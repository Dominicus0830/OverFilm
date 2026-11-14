//! Hardware H.264 export on Windows through Media Foundation's encoder MFTs: the video encoder of
//! AMD (AMF), Intel (Quick Sync) and NVIDIA GPUs, whichever the system's drivers register. It is
//! registered behind NVENC ([`crate::nvenc`]), so an NVIDIA GPU keeps its tuned NVENC path and this
//! one serves the others (an AMD Radeon in particular).
//!
//! ```text
//! RGBA (the export pipeline) ──► BT.709 limited 4:2:0 (the software encoder's conversion, on the
//!    render threads) ──► NV12 ──► encoder MFT (system memory) ──► Annex B ──► avcC + length-prefixed samples
//! ```
//!
//! Like NVENC, it takes an export only when Export ▸ Hardware encoding is Auto and the settings are
//! ones it can write (one pass, VBR or CBR, SDR, progressive, even sizes, MP4 / MOV); otherwise the
//! factory declines and the software encoder runs. B-frames are switched off (`ICodecAPI`), so every
//! sample is presented in decode order. An encoder that fails during the export ends it with an error
//! that says to turn hardware encoding off.
//!
//! The FFI is `media_foundation::encode`; this module is safe code, and its stream handling (the
//! settings it takes, timestamps, Annex B to MP4 samples, NV12) is compiled and tested everywhere.

use filmcraft_export::{BitrateMode, ExportSettings, FieldOrder, Format, H264Pass, H264Profile};
use filmcraft_time::FrameRate;

/// What the encoder is asked to write.
#[derive(Clone, Debug, PartialEq)]
pub struct EncodeConfig {
    pub width: u32,
    pub height: u32,
    /// Frame rate numerator / denominator.
    pub fps: (u32, u32),
    pub bitrate_kbps: u32,
    pub max_bitrate_kbps: u32,
    pub cbr: bool,
    /// Frames between keyframes.
    pub keyint: u32,
    /// `profile_idc`: 66 Baseline, 77 Main, 100 High.
    pub profile: u32,
    /// Level × 10, or the encoder's choice.
    pub level: Option<u8>,
    /// Pixel aspect ratio.
    pub sar: (u32, u32),
}

/// The encoder configuration for an export, or why the hardware path does not take it (the
/// software encoder does).
pub fn config(format: Format, w: u32, h: u32, rate: FrameRate, s: &ExportSettings) -> Result<EncodeConfig, String> {
    if format != Format::H264 || s.format.is_mxf() {
        return Err("only MP4 / MOV H.264 exports".into());
    }
    if s.signal.is_hdr() {
        return Err("HDR".into());
    }
    if !matches!(s.h264_pass, H264Pass::Single) || s.bitrate_mode == BitrateMode::Vbr2Pass {
        return Err("two-pass VBR".into());
    }
    if s.bitrate_mode == BitrateMode::Crf {
        return Err("CRF (constant quality)".into());
    }
    if s.field_order != FieldOrder::Progressive {
        return Err("interlaced output".into());
    }
    if w == 0 || h == 0 || !w.is_multiple_of(2) || !h.is_multiple_of(2) {
        return Err("hardware encoding needs nonzero even dimensions".into());
    }
    if w > 8192 || h > 8192 {
        return Err(format!("{w}x{h} is larger than hardware encoders take"));
    }
    let (Ok(num), Ok(den)) = (u32::try_from(rate.num), u32::try_from(rate.den)) else {
        return Err("frame rate".into());
    };
    if num == 0 || den == 0 {
        return Err("frame rate".into());
    }
    let kbps = s.bitrate_kbps.max(100);
    Ok(EncodeConfig {
        width: w,
        height: h,
        fps: (num, den),
        bitrate_kbps: kbps,
        max_bitrate_kbps: s.max_bitrate_kbps.filter(|m| *m >= kbps).unwrap_or_else(|| kbps.saturating_add(kbps / 2)),
        cbr: s.bitrate_mode == BitrateMode::Cbr,
        keyint: s.keyframe_distance.filter(|k| *k > 0).unwrap_or_else(|| (f64::from(num) / f64::from(den) * 2.0).round().clamp(1.0, 600.0) as u32),
        profile: match s.h264_profile {
            H264Profile::Baseline => 66,
            H264Profile::Main => 77,
            H264Profile::High => 100,
        },
        level: s.h264_level,
        sar: s.pixel_aspect.map_or((1, 1), |(n, d)| (n.clamp(1, 65535), d.clamp(1, 65535))),
    })
}

/// The presentation time of picture `index` in 100 ns units (saturating on hostile values).
pub fn picture_time(index: u64, fps: (u32, u32)) -> i64 {
    let (num, den) = (u128::from(fps.0.max(1)), u128::from(fps.1.max(1)));
    let t = u128::from(index).saturating_mul(10_000_000).saturating_mul(den) / num;
    i64::try_from(t).unwrap_or(i64::MAX)
}

/// One picture's duration in 100 ns units.
pub fn picture_duration(fps: (u32, u32)) -> i64 {
    picture_time(1, fps).max(1)
}

/// The picture a 100 ns time belongs to (the nearest one: encoders round their timestamps).
pub fn picture_at(time: i64, fps: (u32, u32)) -> i64 {
    let (num, den) = (i128::from(fps.0.max(1)), i128::from(fps.1.max(1)));
    let scaled = i128::from(time).saturating_mul(num);
    let unit = den.saturating_mul(10_000_000);
    let half = unit / 2;
    let rounded = if scaled >= 0 { (scaled + half) / unit } else { (scaled - half) / unit };
    i64::try_from(rounded).unwrap_or(if rounded < 0 { i64::MIN } else { i64::MAX })
}

/// The NAL units of an Annex B stream (start codes and trailing zero bytes removed).
pub fn nal_units(annex_b: &[u8]) -> Vec<&[u8]> {
    let mut starts = Vec::new();
    let mut i = 0usize;
    while i + 3 <= annex_b.len() {
        if annex_b.get(i..i + 3) == Some(&[0, 0, 1]) {
            starts.push(i + 3);
            i += 3;
        } else {
            i += 1;
        }
    }
    let mut out = Vec::with_capacity(starts.len());
    for (k, &s) in starts.iter().enumerate() {
        let end = starts.get(k + 1).map_or(annex_b.len(), |next| next - 3);
        let Some(mut nal) = annex_b.get(s..end) else { continue };
        while let [rest @ .., 0] = nal {
            nal = rest;
        }
        if !nal.is_empty() {
            out.push(nal);
        }
    }
    out
}

/// The parameter sets seen in the stream (the first of each).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ParamSets {
    pub sps: Option<Vec<u8>>,
    pub pps: Option<Vec<u8>>,
}

impl ParamSets {
    pub fn complete(&self) -> bool {
        self.sps.is_some() && self.pps.is_some()
    }

    /// Take the SPS / PPS of an Annex B blob (an MFT's sequence header).
    pub fn learn(&mut self, annex_b: &[u8]) {
        for nal in nal_units(annex_b) {
            self.keep(nal);
        }
    }

    fn keep(&mut self, nal: &[u8]) -> bool {
        match nal.first().map(|b| b & 0x1f) {
            Some(7) => {
                self.sps.get_or_insert_with(|| nal.to_vec());
                true
            }
            Some(8) => {
                self.pps.get_or_insert_with(|| nal.to_vec());
                true
            }
            _ => false,
        }
    }
}

/// An Annex B access unit as an MP4 sample (4-byte lengths): parameter sets go to `params` (the
/// sample entry carries them) and access unit delimiters are dropped. The second value says whether
/// it holds an IDR slice.
pub fn to_sample(annex_b: &[u8], params: &mut ParamSets) -> (Vec<u8>, bool) {
    let mut out = Vec::with_capacity(annex_b.len().saturating_add(16));
    let mut idr = false;
    for nal in nal_units(annex_b) {
        if params.keep(nal) {
            continue;
        }
        let kind = nal.first().map_or(0, |b| b & 0x1f);
        if kind == 9 {
            continue;
        }
        idr |= kind == 5;
        let Ok(len) = u32::try_from(nal.len()) else { continue };
        out.extend_from_slice(&len.to_be_bytes());
        out.extend_from_slice(nal);
    }
    (out, idr)
}

/// Planar 4:2:0 (`y`, `u`, `v`; even `w × h`) as NV12 (luma, then interleaved Cb Cr) in `out`.
pub fn nv12(y: &[u8], u: &[u8], v: &[u8], w: usize, h: usize, out: &mut Vec<u8>) -> Result<(), String> {
    let luma = w.checked_mul(h).ok_or("picture size")?;
    let chroma = (w / 2).checked_mul(h / 2).ok_or("picture size")?;
    let (Some(y), Some(u), Some(v)) = (y.get(..luma), u.get(..chroma), v.get(..chroma)) else {
        return Err("short picture planes".into());
    };
    out.clear();
    out.reserve(luma + chroma * 2);
    out.extend_from_slice(y);
    for (a, b) in u.iter().zip(v) {
        out.push(*a);
        out.push(*b);
    }
    Ok(())
}

#[cfg(target_os = "windows")]
pub use self::export::factory;

#[cfg(target_os = "windows")]
mod export {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex, PoisonError};

    use filmcraft_export::{EncodedPacket, EncoderFrame, ExportError, ExportSettings, Format, HardwareEncoding, PrepareAhead, Result, VideoEncoder};
    use filmcraft_isobmff::{AvcConfig, SampleEntry};
    use filmcraft_time::FrameRate;

    use super::{EncodeConfig, ParamSets, nv12, picture_at, to_sample};
    use crate::media_foundation::{MfEncoder, RawPacket, open_h264_encoder};

    const HINT: &str = " (turn Export > Hardware encoding off, or OverFilm > Render fast off, to use the software encoder)";

    /// Pictures converted ahead on the render threads, by index.
    type Ready = Arc<Mutex<HashMap<u64, Vec<u8>>>>;

    struct MfH264 {
        enc: MfEncoder,
        w: u32,
        h: u32,
        rate: FrameRate,
        fps: (u32, u32),
        params: ParamSets,
        /// Samples written so far: the decode index of the next one.
        written: i64,
        ready: Ready,
        buf: Vec<u8>,
    }

    fn convert(f: &EncoderFrame, out: &mut Vec<u8>) -> std::result::Result<(), String> {
        let (w, h) = (f.width as usize, f.height as usize);
        let (mut y, mut u, mut v) = (Vec::new(), Vec::new(), Vec::new());
        filmcraft_export::rgba_to_yuv420_8(f.rgba, w, h, &mut y, &mut u, &mut v);
        nv12(&y, &u, &v, w, h, out)
    }

    impl MfH264 {
        fn packets(&mut self, raw: Vec<RawPacket>) -> Result<Vec<EncodedPacket>> {
            let den = i64::from(self.fps.1);
            let mut out = Vec::with_capacity(raw.len());
            for p in raw {
                let (data, idr) = to_sample(&p.data, &mut self.params);
                if data.is_empty() {
                    continue;
                }
                let pts = picture_at(p.time, self.fps);
                let dts = self.written;
                self.written = self.written.saturating_add(1);
                let offset = pts.saturating_sub(dts).saturating_mul(den);
                if offset < 0 {
                    return Err(ExportError::Encode(format!("{}: wrote pictures out of order{HINT}", self.enc.name())));
                }
                out.push(EncodedPacket {
                    data,
                    key: idr || p.key,
                    duration: u32::try_from(self.rate.den).unwrap_or(u32::MAX),
                    composition_offset: i32::try_from(offset).unwrap_or(i32::MAX),
                });
            }
            Ok(out)
        }

        fn entry(&self) -> SampleEntry {
            let (w, h) = (u16::try_from(self.w).unwrap_or(u16::MAX), u16::try_from(self.h).unwrap_or(u16::MAX));
            let sps = self.params.sps.clone().into_iter().collect();
            let pps = self.params.pps.clone().into_iter().collect();
            let mut e = SampleEntry::avc(AvcConfig::new(sps, pps, 4), w, h);
            filmcraft_export::ColorSignal::default().apply_to(&mut e, false);
            e
        }
    }

    impl VideoEncoder for MfH264 {
        fn sample_entry(&self) -> SampleEntry {
            self.entry()
        }

        fn final_sample_entry(&self) -> Option<SampleEntry> {
            Some(self.entry())
        }

        fn timescale(&self) -> u32 {
            u32::try_from(self.rate.num).unwrap_or(u32::MAX)
        }

        fn encode(&mut self, f: &EncoderFrame) -> Result<Vec<EncodedPacket>> {
            if f.width != self.w || f.height != self.h {
                return Err(ExportError::Encode(format!("a {}x{} picture for a {}x{} hardware encoder", f.width, f.height, self.w, self.h)));
            }
            let ready = {
                let mut ready = self.ready.lock().unwrap_or_else(PoisonError::into_inner);
                let mine = ready.remove(&f.index);
                ready.retain(|&i, _| i > f.index);
                mine
            };
            let want = (self.w as usize) * (self.h as usize) * 3 / 2;
            match ready.filter(|b| b.len() == want) {
                Some(b) => self.buf = b,
                None => {
                    let buf = &mut self.buf;
                    filmcraft_export::timed(filmcraft_export::Stage::Convert, || convert(f, buf)).map_err(ExportError::Encode)?;
                }
            }
            let raw = self.enc.encode(&self.buf, f.index).map_err(|e| ExportError::Encode(format!("{e}{HINT}")))?;
            filmcraft_export::note_hw_encode_frame();
            if !self.params.complete()
                && let Some(header) = self.enc.sequence_header()
            {
                self.params.learn(&header);
            }
            self.packets(raw)
        }

        fn flush(&mut self) -> Result<Vec<EncodedPacket>> {
            let raw = self.enc.flush().map_err(|e| ExportError::Encode(format!("{e}{HINT}")))?;
            let out = self.packets(raw)?;
            if !self.params.complete() {
                return Err(ExportError::Encode(format!("{}: the stream has no parameter sets{HINT}", self.enc.name())));
            }
            Ok(out)
        }

        fn prepare_ahead(&self) -> Option<PrepareAhead> {
            let ready = self.ready.clone();
            Some(Arc::new(move |f: &EncoderFrame| {
                let mut out = Vec::new();
                if filmcraft_export::timed(filmcraft_export::Stage::Convert, || convert(f, &mut out)).is_ok() {
                    ready.lock().unwrap_or_else(PoisonError::into_inner).insert(f.index, out);
                }
            }))
        }
    }

    /// The Export encoder factory: in front of the software H.264 encoder, behind NVENC.
    pub fn factory(format: Format, w: u32, h: u32, rate: FrameRate, s: &ExportSettings) -> Option<Result<Box<dyn VideoEncoder>>> {
        if s.hardware_encoding != HardwareEncoding::Auto || format != Format::H264 {
            return None;
        }
        let declined = |why: &str| {
            log::info!("Media Foundation hardware encoding declined: {why}");
            filmcraft_export::note_hw_encode_declined();
            None
        };
        let cfg: EncodeConfig = match super::config(format, w, h, rate, s) {
            Ok(c) => c,
            Err(why) => return declined(&why),
        };
        match open_h264_encoder(&cfg) {
            Ok(enc) => {
                log::info!("hardware H.264 encoding with {}", enc.name());
                filmcraft_export::note_hw_encode_session();
                let mut params = ParamSets::default();
                if let Some(header) = enc.sequence_header() {
                    params.learn(&header);
                }
                Some(Ok(Box::new(MfH264 { enc, w, h, rate, fps: cfg.fps, params, written: 0, ready: Default::default(), buf: Vec::new() })))
            }
            Err(why) => declined(&why),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use filmcraft_export::{ColorSignal, HardwareEncoding};

    #[test]
    fn takes_plain_h264_and_declines_the_rest() {
        let ok = ExportSettings { format: Format::H264, hardware_encoding: HardwareEncoding::Auto, bitrate_kbps: 12_000, ..Default::default() };
        let c = config(Format::H264, 2560, 1440, FrameRate::FPS_60, &ok).unwrap();
        assert_eq!((c.width, c.height, c.fps, c.bitrate_kbps, c.max_bitrate_kbps, c.keyint, c.profile), (2560, 1440, (60, 1), 12_000, 18_000, 120, 100));
        assert_eq!(config(Format::H264, 1920, 1080, FrameRate::FPS_29_97, &ok).unwrap().fps, (30_000, 1001));
        for (what, s) in [
            ("HDR", ExportSettings { signal: ColorSignal::PQ, ..ok.clone() }),
            ("two-pass", ExportSettings { bitrate_mode: BitrateMode::Vbr2Pass, ..ok.clone() }),
            ("CRF", ExportSettings { bitrate_mode: BitrateMode::Crf, ..ok.clone() }),
            ("analysis pass", ExportSettings { h264_pass: H264Pass::First, ..ok.clone() }),
            ("interlaced", ExportSettings { field_order: FieldOrder::UpperFirst, ..ok.clone() }),
            ("MXF", ExportSettings { format: Format::MxfOp1a, ..ok.clone() }),
        ] {
            assert!(config(Format::H264, 1280, 720, FrameRate::FPS_24, &s).is_err(), "{what}");
        }
        assert!(config(Format::ProRes, 1280, 720, FrameRate::FPS_24, &ok).is_err());
        for (w, h) in [(641, 360), (0, 360), (640, 0), (9000, 64)] {
            assert!(config(Format::H264, w, h, FrameRate::FPS_24, &ok).is_err(), "{w}x{h}");
        }
        for rate in [FrameRate { num: 0, den: 1 }, FrameRate { num: 24, den: 0 }, FrameRate { num: -24, den: 1 }, FrameRate { num: i64::MAX, den: 1 }] {
            assert!(config(Format::H264, 1280, 720, rate, &ok).is_err(), "{rate:?}");
        }
        let huge = ExportSettings { bitrate_kbps: u32::MAX, ..ok };
        assert_eq!(config(Format::H264, 64, 64, FrameRate::FPS_24, &huge).unwrap().max_bitrate_kbps, u32::MAX);
    }

    #[test]
    fn timestamps_round_trip_and_saturate() {
        for fps in [(24, 1), (30_000, 1001), (60, 1), (24_000, 1001)] {
            for i in [0u64, 1, 2, 59, 1000, 123_456] {
                assert_eq!(picture_at(picture_time(i, fps), fps), i as i64, "{fps:?} {i}");
                // an encoder that rounds its timestamps by a few 100 ns units still lands on the picture
                assert_eq!(picture_at(picture_time(i, fps) + 3, fps), i as i64);
            }
        }
        assert_eq!(picture_duration((30_000, 1001)), 333_666);
        assert_eq!(picture_time(u64::MAX, (1, u32::MAX)), i64::MAX);
        assert_eq!(picture_time(5, (0, 0)), 50_000_000);
        assert_eq!(picture_at(i64::MAX, (u32::MAX, 1)), i64::MAX);
        assert_eq!(picture_at(i64::MIN, (u32::MAX, 1)), i64::MIN);
    }

    #[test]
    fn annex_b_becomes_mp4_samples_and_parameter_sets() {
        let sps = [0x67, 0x64, 0x00, 0x28, 0xac];
        let pps = [0x68, 0xee, 0x3c, 0x80];
        let idr = [0x65, 0x88, 0x84, 0x00, 0x00, 0x03, 0x01];
        let mut au = vec![0, 0, 0, 1, 0x09, 0x10];
        for nal in [&sps[..], &pps[..], &idr[..]] {
            au.extend_from_slice(&[0, 0, 0, 1]);
            au.extend_from_slice(nal);
        }
        au.push(0); // trailing zero byte
        let mut params = ParamSets::default();
        let (sample, key) = to_sample(&au, &mut params);
        assert!(key);
        assert_eq!(params, ParamSets { sps: Some(sps.to_vec()), pps: Some(pps.to_vec()) });
        let mut want = (idr.len() as u32).to_be_bytes().to_vec();
        want.extend_from_slice(&idr);
        assert_eq!(sample, want, "only the slice, length-prefixed (AUD and parameter sets out)");
        // a P slice with a 3-byte start code; the first parameter sets are kept
        let (p, key) = to_sample(&[0, 0, 1, 0x67, 9, 0, 0, 1, 0x41, 0x9a, 0x02], &mut params);
        assert!(!key);
        assert_eq!(p, [0, 0, 0, 3, 0x41, 0x9a, 0x02]);
        assert_eq!(params.sps.as_deref(), Some(&sps[..]));
        // hostile input: nothing, garbage, start codes only
        for junk in [&[][..], &[1, 2, 3][..], &[0, 0, 1][..], &[0, 0, 1, 0, 0, 1][..], &[0, 0, 0, 0][..]] {
            let (s, k) = to_sample(junk, &mut ParamSets::default());
            assert!(s.is_empty() && !k, "{junk:?}");
        }
        let mut learnt = ParamSets::default();
        learnt.learn(&au);
        assert!(learnt.complete());
    }

    #[test]
    fn nv12_interleaves_chroma_and_rejects_short_planes() {
        let (y, u, v) = ([1u8, 2, 3, 4, 5, 6, 7, 8], [10u8, 11], [20u8, 21]);
        let mut out = Vec::new();
        nv12(&y, &u, &v, 4, 2, &mut out).unwrap();
        assert_eq!(out, [1, 2, 3, 4, 5, 6, 7, 8, 10, 20, 11, 21]);
        assert!(nv12(&y, &u[..1], &v, 4, 2, &mut out).is_err());
        assert!(nv12(&y, &u, &v, usize::MAX, 2, &mut out).is_err());
    }
}
