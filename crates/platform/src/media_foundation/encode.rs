//! The Media Foundation side of the Windows hardware H.264 encoder: finding a vendor's hardware
//! encoder MFT (AMD's AMF, Intel's Quick Sync, NVIDIA's NVENC MFT...), configuring it, and driving it
//! with its event queue (`METransformNeedInput` / `METransformHaveOutput`), one NV12 picture in
//! system memory at a time.
//!
//! FFI module (docs/adr/0001-platform-ffi.md): every `unsafe` block has a `// SAFETY:` comment,
//! the public items are safe and return `Result<_, String>`. No Rust callback is registered: the
//! event queue is polled ([`IMFMediaEventGenerator::GetEvent`] with `MF_EVENT_FLAG_NO_WAIT`), with
//! a deadline, so a driver that stops answering ends the export with an error instead of hanging it.

use std::mem::ManuallyDrop;
use std::time::{Duration, Instant};

use windows::Win32::Media::MediaFoundation::{
    CODECAPI_AVEncCommonMaxBitRate, CODECAPI_AVEncCommonMeanBitRate, CODECAPI_AVEncCommonRateControlMode, CODECAPI_AVEncMPVDefaultBPictureCount,
    CODECAPI_AVEncMPVGOPSize, ICodecAPI, IMFActivate, IMFMediaEventGenerator, IMFMediaType, IMFSample, IMFShutdown, IMFTransform, METransformDrainComplete,
    METransformHaveOutput, METransformNeedInput, MF_E_NO_EVENTS_AVAILABLE, MF_E_NO_MORE_TYPES, MF_E_TRANSFORM_NEED_MORE_INPUT, MF_E_TRANSFORM_STREAM_CHANGE,
    MF_EVENT_FLAG_NO_WAIT, MF_MT_AVG_BITRATE, MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE, MF_MT_INTERLACE_MODE, MF_MT_MAJOR_TYPE, MF_MT_MPEG_SEQUENCE_HEADER,
    MF_MT_MPEG2_LEVEL, MF_MT_MPEG2_PROFILE, MF_MT_PIXEL_ASPECT_RATIO, MF_MT_SUBTYPE, MF_MT_TRANSFER_FUNCTION, MF_MT_VIDEO_NOMINAL_RANGE, MF_MT_VIDEO_PRIMARIES,
    MF_MT_YUV_MATRIX, MF_TRANSFORM_ASYNC, MF_TRANSFORM_ASYNC_UNLOCK, MFMediaType_Video, MFNominalRange_16_235, MFSampleExtension_CleanPoint,
    MFT_MESSAGE_COMMAND_DRAIN, MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, MFT_MESSAGE_NOTIFY_END_OF_STREAM, MFT_MESSAGE_NOTIFY_END_STREAMING,
    MFT_MESSAGE_NOTIFY_START_OF_STREAM, MFT_OUTPUT_DATA_BUFFER, MFT_OUTPUT_STREAM_PROVIDES_SAMPLES, MFVideoFormat_H264, MFVideoFormat_NV12,
    MFVideoInterlace_Progressive, MFVideoPrimaries_BT709, MFVideoTransFunc_709, MFVideoTransferMatrix_BT709, eAVEncCommonRateControlMode_CBR,
    eAVEncCommonRateControlMode_PeakConstrainedVBR,
};
use windows::Win32::System::Variant::{VARIANT, VARIANT_0, VARIANT_0_0, VARIANT_0_0_0, VT_UI4};
use windows::core::{GUID, Interface};

use super::gpu::ensure_com;
use super::mft::{MfApi, friendly_name};
use crate::mf_h264::{EncodeConfig, picture_duration, picture_time};

/// How long the encoder may take to ask for the next picture or to finish draining before the
/// export gives up on it (a lost device, a hung driver).
const EVENT_DEADLINE: Duration = Duration::from_secs(20);
/// Upper bound on the events handled per call: a misbehaving MFT cannot make a call loop.
const MAX_EVENTS_PER_CALL: usize = 4096;

/// One encoded access unit as the MFT writes it.
pub struct RawPacket {
    /// Annex B (start codes).
    pub data: Vec<u8>,
    /// Presentation time (100 ns units), as given to the input picture.
    pub time: i64,
    /// The MFT marked it a clean point (a keyframe).
    pub key: bool,
}

/// A hardware H.264 encoder MFT configured for one export.
pub struct MfEncoder {
    api: &'static MfApi,
    transform: IMFTransform,
    events: IMFMediaEventGenerator,
    name: String,
    /// `METransformNeedInput` events not yet answered with a picture.
    need_input: u32,
    /// The MFT allocates its output samples (else they are made here, `out_size` bytes).
    provides_samples: bool,
    out_size: u32,
    fps: (u32, u32),
    drained: bool,
}

// SAFETY: the hardware encoder MFTs are free-threaded (asynchronous MFTs must be, as their events
// come from their own work queue); an `MfEncoder` is used through `&mut` by one thread at a time,
// and every call makes sure that thread has COM initialised.
unsafe impl Send for MfEncoder {}

/// A `VT_UI4` VARIANT (an `ICodecAPI` value). It holds no resource, so it needs no `VariantClear`.
fn variant_u32(value: u32) -> VARIANT {
    VARIANT {
        Anonymous: VARIANT_0 {
            Anonymous: ManuallyDrop::new(VARIANT_0_0 { vt: VT_UI4, wReserved1: 0, wReserved2: 0, wReserved3: 0, Anonymous: VARIANT_0_0_0 { ulVal: value } }),
        },
    }
}

/// `(a << 32) | b`, the packing of Media Foundation's size and ratio attributes.
fn pack(a: u32, b: u32) -> u64 {
    (u64::from(a) << 32) | u64::from(b)
}

/// What one poll of the event queue said.
enum Event {
    NeedInput,
    HaveOutput,
    DrainComplete,
    Other,
}

impl MfEncoder {
    /// Create the first hardware encoder MFT that takes `cfg`. Errors (and so the software encoder
    /// is used) when the system has none, or none accepts the settings.
    pub fn new(api: &'static MfApi, cfg: &EncodeConfig) -> Result<MfEncoder, String> {
        ensure_com()?;
        let candidates = api.hardware_encoders(MFVideoFormat_H264)?;
        if candidates.is_empty() {
            return Err("no hardware H.264 encoder MFT (no AMD, Intel or NVIDIA video encoder driver)".into());
        }
        let mut reasons = Vec::new();
        for a in &candidates {
            match Self::configure(api, a, cfg) {
                Ok(e) => return Ok(e),
                Err(e) => reasons.push(e),
            }
        }
        Err(reasons.join("; "))
    }

    /// The MFT's name (`AMDh264Encoder`, `Intel® Quick Sync Video H.264 Encoder MFT`...).
    pub fn name(&self) -> &str {
        &self.name
    }

    fn configure(api: &'static MfApi, activate: &IMFActivate, cfg: &EncodeConfig) -> Result<MfEncoder, String> {
        let name = friendly_name(activate).unwrap_or_else(|| "a hardware encoder MFT".into());
        // SAFETY: plain COM calls on live interfaces.
        let transform: IMFTransform = unsafe { activate.ActivateObject() }.map_err(|e| format!("{name}: cannot be created: {e}"))?;
        // SAFETY: plain COM calls on live interfaces.
        let events = unsafe {
            let attrs = transform.GetAttributes().map_err(|e| format!("{name}: no attributes: {e}"))?;
            if attrs.GetUINT32(&MF_TRANSFORM_ASYNC).unwrap_or(0) == 0 {
                return Err(format!("{name}: synchronous encoder MFTs are not supported"));
            }
            attrs.SetUINT32(&MF_TRANSFORM_ASYNC_UNLOCK, 1).map_err(|e| format!("{name}: cannot be unlocked: {e}"))?;
            transform.cast::<IMFMediaEventGenerator>().map_err(|e| format!("{name}: has no event queue: {e}"))?
        };
        let enc = MfEncoder {
            api,
            transform,
            events,
            name,
            need_input: 0,
            provides_samples: false,
            out_size: 0,
            fps: (cfg.fps.0.max(1), cfg.fps.1.max(1)),
            drained: false,
        };
        enc.set_codec_api(cfg);
        enc.set_output_type(cfg)?;
        enc.set_input_type(cfg)?;
        // the rate settings again: some MFTs take them only once the types are set
        enc.set_codec_api(cfg);
        let mut enc = enc;
        // SAFETY: plain COM calls on live interfaces.
        unsafe {
            let info = enc.transform.GetOutputStreamInfo(0).map_err(|e| format!("{}: no output stream info: {e}", enc.name))?;
            enc.provides_samples = info.dwFlags & MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 as u32 != 0;
            let frame = u64::from(cfg.width).saturating_mul(u64::from(cfg.height)).saturating_mul(3) / 2;
            enc.out_size = info.cbSize.max(u32::try_from(frame).unwrap_or(u32::MAX)).max(64 * 1024);
            enc.transform.ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0).map_err(|e| format!("{}: cannot start: {e}", enc.name))?;
            enc.transform.ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0).map_err(|e| format!("{}: cannot start: {e}", enc.name))?;
        }
        Ok(enc)
    }

    /// Rate control, keyframe distance and no B-frames through `ICodecAPI`, where the MFT has it
    /// (best effort: the output type's bitrate stands for an MFT that refuses).
    fn set_codec_api(&self, cfg: &EncodeConfig) {
        let Ok(api) = self.transform.cast::<ICodecAPI>() else { return };
        let mode = if cfg.cbr { eAVEncCommonRateControlMode_CBR } else { eAVEncCommonRateControlMode_PeakConstrainedVBR };
        let mut values: Vec<(GUID, u32)> = vec![
            (CODECAPI_AVEncCommonRateControlMode, mode.0 as u32),
            (CODECAPI_AVEncCommonMeanBitRate, cfg.bitrate_kbps.saturating_mul(1000)),
            (CODECAPI_AVEncMPVGOPSize, cfg.keyint.max(1)),
            (CODECAPI_AVEncMPVDefaultBPictureCount, 0),
        ];
        if !cfg.cbr {
            values.push((CODECAPI_AVEncCommonMaxBitRate, cfg.max_bitrate_kbps.saturating_mul(1000)));
        }
        for (key, value) in values {
            let v = variant_u32(value);
            // SAFETY: a plain COM call; both pointers refer to live locals for the call's duration.
            if let Err(e) = unsafe { api.SetValue(&key, &v) } {
                log::debug!("{}: ICodecAPI {key:?} = {value} refused: {e}", self.name);
            }
        }
    }

    fn base_type(&self, cfg: &EncodeConfig, subtype: &GUID) -> Result<IMFMediaType, String> {
        let t = self.api.media_type()?;
        // SAFETY: plain COM calls on a live interface.
        unsafe {
            t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video).map_err(|e| e.to_string())?;
            t.SetGUID(&MF_MT_SUBTYPE, subtype).map_err(|e| e.to_string())?;
        }
        self.describe(&t, cfg)?;
        Ok(t)
    }

    /// Size, rate, scan and colour (BT.709 limited range, what the export's conversion writes).
    fn describe(&self, t: &IMFMediaType, cfg: &EncodeConfig) -> Result<(), String> {
        // SAFETY: plain COM calls on a live interface.
        unsafe {
            t.SetUINT64(&MF_MT_FRAME_SIZE, pack(cfg.width, cfg.height)).map_err(|e| e.to_string())?;
            t.SetUINT64(&MF_MT_FRAME_RATE, pack(self.fps.0, self.fps.1)).map_err(|e| e.to_string())?;
            t.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32).map_err(|e| e.to_string())?;
            t.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack(cfg.sar.0.max(1), cfg.sar.1.max(1))).map_err(|e| e.to_string())?;
            // colour description: best effort (an MFT that has no such attribute keeps its default, BT.709)
            let _ = t.SetUINT32(&MF_MT_YUV_MATRIX, MFVideoTransferMatrix_BT709.0 as u32);
            let _ = t.SetUINT32(&MF_MT_VIDEO_NOMINAL_RANGE, MFNominalRange_16_235.0 as u32);
            let _ = t.SetUINT32(&MF_MT_VIDEO_PRIMARIES, MFVideoPrimaries_BT709.0 as u32);
            let _ = t.SetUINT32(&MF_MT_TRANSFER_FUNCTION, MFVideoTransFunc_709.0 as u32);
        }
        Ok(())
    }

    fn set_output_type(&self, cfg: &EncodeConfig) -> Result<(), String> {
        let t = self.base_type(cfg, &MFVideoFormat_H264)?;
        // SAFETY: plain COM calls on live interfaces.
        unsafe {
            t.SetUINT32(&MF_MT_AVG_BITRATE, cfg.bitrate_kbps.saturating_mul(1000)).map_err(|e| e.to_string())?;
            t.SetUINT32(&MF_MT_MPEG2_PROFILE, cfg.profile).map_err(|e| e.to_string())?;
            if let Some(level) = cfg.level {
                let _ = t.SetUINT32(&MF_MT_MPEG2_LEVEL, u32::from(level));
            }
            self.transform.SetOutputType(0, &t, 0).map_err(|e| format!("{}: does not take these H.264 settings: {e}", self.name))
        }
    }

    /// NV12 input: the MFT's own NV12 type (with its strides and alignment) completed with the
    /// picture's description, else one made here.
    fn set_input_type(&self, cfg: &EncodeConfig) -> Result<(), String> {
        for i in 0..64 {
            // SAFETY: plain COM calls on live interfaces.
            let t = match unsafe { self.transform.GetInputAvailableType(0, i) } {
                Ok(t) => t,
                Err(e) if e.code() == MF_E_NO_MORE_TYPES => break,
                Err(_) => break,
            };
            // SAFETY: plain COM call on a live interface.
            if unsafe { t.GetGUID(&MF_MT_SUBTYPE) }.ok() != Some(MFVideoFormat_NV12) {
                continue;
            }
            self.describe(&t, cfg)?;
            // SAFETY: plain COM call on live interfaces.
            if unsafe { self.transform.SetInputType(0, &t, 0) }.is_ok() {
                return Ok(());
            }
        }
        let t = self.base_type(cfg, &MFVideoFormat_NV12)?;
        // SAFETY: plain COM call on live interfaces.
        unsafe { self.transform.SetInputType(0, &t, 0) }.map_err(|e| format!("{}: does not take NV12 pictures of this size: {e}", self.name))
    }

    /// The parameter sets the MFT announces on its output type (Annex B), when it does.
    pub fn sequence_header(&self) -> Option<Vec<u8>> {
        // SAFETY: plain COM calls on live interfaces; `GetBlob` writes at most `buf.len()` bytes.
        unsafe {
            let t = self.transform.GetOutputCurrentType(0).ok()?;
            let len = t.GetBlobSize(&MF_MT_MPEG_SEQUENCE_HEADER).ok()?;
            if len == 0 || len > 1 << 16 {
                return None;
            }
            let mut buf = vec![0u8; len as usize];
            let mut got = 0u32;
            t.GetBlob(&MF_MT_MPEG_SEQUENCE_HEADER, &mut buf, Some(&mut got)).ok()?;
            buf.truncate(got as usize);
            Some(buf)
        }
    }

    /// The next event, or `None` when the queue is empty.
    fn try_event(&self) -> Result<Option<Event>, String> {
        // SAFETY: plain COM calls on live interfaces.
        unsafe {
            match self.events.GetEvent(MF_EVENT_FLAG_NO_WAIT) {
                Ok(ev) => {
                    if let Ok(status) = ev.GetStatus()
                        && status.is_err()
                    {
                        return Err(format!("{}: reported an error ({status:?})", self.name));
                    }
                    let kind = ev.GetType().map_err(|e| format!("{}: unreadable event: {e}", self.name))?;
                    Ok(Some(match kind {
                        k if k == METransformNeedInput.0 as u32 => Event::NeedInput,
                        k if k == METransformHaveOutput.0 as u32 => Event::HaveOutput,
                        k if k == METransformDrainComplete.0 as u32 => Event::DrainComplete,
                        _ => Event::Other,
                    }))
                }
                Err(e) if e.code() == MF_E_NO_EVENTS_AVAILABLE => Ok(None),
                Err(e) => Err(format!("{}: event queue failed: {e}", self.name)),
            }
        }
    }

    /// The next event, waiting for it up to the deadline.
    fn wait_event(&self, deadline: Instant) -> Result<Event, String> {
        loop {
            if let Some(ev) = self.try_event()? {
                return Ok(ev);
            }
            if Instant::now() >= deadline {
                return Err(format!("{}: stopped answering (no event for {} s)", self.name, EVENT_DEADLINE.as_secs()));
            }
            std::thread::sleep(Duration::from_micros(250));
        }
    }

    /// Handle one event: count input requests, collect output.
    fn handle(&mut self, ev: Event, out: &mut Vec<RawPacket>) -> Result<bool, String> {
        match ev {
            Event::NeedInput => self.need_input = self.need_input.saturating_add(1),
            Event::HaveOutput => {
                if let Some(p) = self.take_output()? {
                    out.push(p);
                }
            }
            Event::DrainComplete => return Ok(true),
            Event::Other => {}
        }
        Ok(false)
    }

    /// Encode one NV12 picture (`width × height` luma, then interleaved chroma) as picture `index`.
    /// Returns the access units the MFT finished meanwhile (it holds a few back).
    pub fn encode(&mut self, nv12: &[u8], index: u64) -> Result<Vec<RawPacket>, String> {
        ensure_com()?;
        if self.drained {
            return Err(format!("{}: already finished", self.name));
        }
        let mut out = Vec::new();
        let deadline = Instant::now() + EVENT_DEADLINE;
        let mut handled = 0usize;
        while self.need_input == 0 {
            let ev = self.wait_event(deadline)?;
            self.handle(ev, &mut out)?;
            handled += 1;
            if handled > MAX_EVENTS_PER_CALL {
                return Err(format!("{}: never asked for a picture", self.name));
            }
        }
        let sample = self.api.timed_sample(nv12, picture_time(index, self.fps), picture_duration(self.fps))?;
        // SAFETY: plain COM call on live interfaces.
        unsafe { self.transform.ProcessInput(0, &sample, 0) }.map_err(|e| format!("{}: ProcessInput failed: {e}", self.name))?;
        self.need_input -= 1;
        for _ in 0..MAX_EVENTS_PER_CALL {
            let Some(ev) = self.try_event()? else { break };
            self.handle(ev, &mut out)?;
        }
        Ok(out)
    }

    /// Finish the stream: the access units still held back.
    pub fn flush(&mut self) -> Result<Vec<RawPacket>, String> {
        ensure_com()?;
        let mut out = Vec::new();
        if self.drained {
            return Ok(out);
        }
        self.drained = true;
        // SAFETY: plain COM calls on a live interface.
        unsafe {
            self.transform.ProcessMessage(MFT_MESSAGE_NOTIFY_END_OF_STREAM, 0).map_err(|e| format!("{}: cannot end the stream: {e}", self.name))?;
            self.transform.ProcessMessage(MFT_MESSAGE_COMMAND_DRAIN, 0).map_err(|e| format!("{}: drain failed: {e}", self.name))?;
        }
        let deadline = Instant::now() + EVENT_DEADLINE;
        for _ in 0..MAX_EVENTS_PER_CALL {
            let ev = self.wait_event(deadline)?;
            if self.handle(ev, &mut out)? {
                return Ok(out);
            }
        }
        Err(format!("{}: never finished draining", self.name))
    }

    /// Take one finished access unit (`METransformHaveOutput`).
    fn take_output(&self) -> Result<Option<RawPacket>, String> {
        let given = if self.provides_samples { None } else { Some(self.api.empty_sample(self.out_size)?) };
        let mut buffers = [MFT_OUTPUT_DATA_BUFFER { dwStreamID: 0, pSample: ManuallyDrop::new(given), dwStatus: 0, pEvents: ManuallyDrop::new(None) }];
        let mut status = 0u32;
        // SAFETY: one output buffer descriptor (with our sample, or none when the MFT provides
        // them) and a live status out-pointer.
        let r = unsafe { self.transform.ProcessOutput(0, &mut buffers, &mut status) };
        // SAFETY: the descriptor's fields were initialised above and are only taken once; dropping
        // them releases the references (ours, or the ones the MFT handed over).
        let (sample, _events) = unsafe { (ManuallyDrop::take(&mut buffers[0].pSample), ManuallyDrop::take(&mut buffers[0].pEvents)) };
        match r {
            Ok(()) => {}
            Err(e) if e.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => return Ok(None),
            Err(e) if e.code() == MF_E_TRANSFORM_STREAM_CHANGE => {
                // the MFT changed its output type (it now knows its parameter sets): take its new one
                // SAFETY: plain COM calls on live interfaces.
                unsafe {
                    let t = self.transform.GetOutputAvailableType(0, 0).map_err(|e| format!("{}: no output type after a change: {e}", self.name))?;
                    self.transform.SetOutputType(0, &t, 0).map_err(|e| format!("{}: refused its own output type: {e}", self.name))?;
                }
                return Ok(None);
            }
            Err(e) => return Err(format!("{}: ProcessOutput failed: {e}", self.name)),
        }
        let sample = sample.ok_or_else(|| format!("{}: produced no sample", self.name))?;
        Ok(Some(read_sample(&sample, &self.name)?))
    }
}

/// The bytes, time and clean-point flag of an output sample.
fn read_sample(sample: &IMFSample, name: &str) -> Result<RawPacket, String> {
    // SAFETY: plain COM calls on live interfaces; `Lock` gives `len` readable bytes valid until
    // `Unlock`, which are copied before it.
    unsafe {
        let time = sample.GetSampleTime().unwrap_or(0);
        let key = sample.GetUINT32(&MFSampleExtension_CleanPoint).unwrap_or(0) != 0;
        let buffer = sample.ConvertToContiguousBuffer().map_err(|e| format!("{name}: unreadable output: {e}"))?;
        let (mut ptr, mut len) = (std::ptr::null_mut::<u8>(), 0u32);
        buffer.Lock(&mut ptr, None, Some(&mut len)).map_err(|e| format!("{name}: cannot lock the output: {e}"))?;
        let data = if ptr.is_null() { Vec::new() } else { std::slice::from_raw_parts(ptr, len as usize).to_vec() };
        let _ = buffer.Unlock();
        Ok(RawPacket { data, time, key })
    }
}

impl Drop for MfEncoder {
    fn drop(&mut self) {
        // SAFETY: plain COM calls on live interfaces; failures leave nothing to clean up.
        unsafe {
            let _ = self.transform.ProcessMessage(MFT_MESSAGE_NOTIFY_END_STREAMING, 0);
            if let Ok(s) = self.transform.cast::<IMFShutdown>() {
                let _ = s.Shutdown();
            }
        }
    }
}
