//! Listening back: plays a short stretch of 16 kHz mono sound on the default output device.
//! The device stream is opened on the first play and kept; a stream that fails is opened again
//! on the next play.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

const RATE: f64 = overfilm_asr::SAMPLE_RATE as f64;

/// What the audio thread reads.
#[derive(Default)]
struct Feed {
    samples: Vec<f32>,
    /// Position in `samples` (fractional: the device runs at its own rate).
    pos: f64,
    /// Source samples per device frame.
    step: f64,
    playing: bool,
}

/// The output stream and what it is playing.
#[derive(Default)]
pub struct Player {
    stream: Option<cpal::Stream>,
    feed: Arc<Mutex<Feed>>,
    failed: Arc<AtomicBool>,
    /// File time (ms) of the first sample being played.
    start_ms: i64,
}

impl Player {
    /// Play `samples` (16 kHz mono) that start at `start_ms` in the file, replacing what plays.
    pub fn play(&mut self, samples: Vec<f32>, start_ms: i64) -> Result<(), String> {
        if self.failed.swap(false, Ordering::SeqCst) {
            self.stream = None;
        }
        if self.stream.is_none() {
            self.open()?;
        }
        let mut f = self.feed.lock().unwrap_or_else(PoisonError::into_inner);
        f.samples = samples;
        f.pos = 0.0;
        f.playing = !f.samples.is_empty();
        self.start_ms = start_ms;
        Ok(())
    }

    pub fn stop(&mut self) {
        self.feed.lock().unwrap_or_else(PoisonError::into_inner).playing = false;
    }

    /// File time (ms) being heard now, while playing.
    pub fn position_ms(&self) -> Option<i64> {
        let f = self.feed.lock().unwrap_or_else(PoisonError::into_inner);
        f.playing.then(|| self.start_ms.saturating_add((f.pos * 1000.0 / RATE) as i64))
    }

    fn open(&mut self) -> Result<(), String> {
        let dev = cpal::default_host().default_output_device().ok_or("소리를 낼 장치가 없습니다")?;
        let cfg = dev.default_output_config().map_err(|e| format!("소리 장치를 열지 못했습니다: {e}"))?;
        let channels = usize::from(cfg.channels());
        let rate = cfg.sample_rate().0;
        if channels == 0 || rate == 0 {
            return Err("소리 장치가 잘못된 형식을 알려 왔습니다".into());
        }
        self.feed.lock().unwrap_or_else(PoisonError::into_inner).step = RATE / f64::from(rate);
        let config: cpal::StreamConfig = cfg.clone().into();
        let stream = match cfg.sample_format() {
            cpal::SampleFormat::F32 => self.build::<f32>(&dev, &config, channels),
            cpal::SampleFormat::I16 => self.build::<i16>(&dev, &config, channels),
            cpal::SampleFormat::U16 => self.build::<u16>(&dev, &config, channels),
            cpal::SampleFormat::I32 => self.build::<i32>(&dev, &config, channels),
            cpal::SampleFormat::I24 => self.build::<cpal::I24>(&dev, &config, channels),
            cpal::SampleFormat::U8 => self.build::<u8>(&dev, &config, channels),
            cpal::SampleFormat::F64 => self.build::<f64>(&dev, &config, channels),
            other => return Err(format!("이 소리 장치 형식은 지원하지 않습니다 ({other:?})")),
        }
        .map_err(|e| format!("소리 장치를 열지 못했습니다: {e}"))?;
        stream.play().map_err(|e| format!("소리를 재생하지 못했습니다: {e}"))?;
        self.stream = Some(stream);
        Ok(())
    }

    fn build<T>(&self, dev: &cpal::Device, config: &cpal::StreamConfig, channels: usize) -> Result<cpal::Stream, cpal::BuildStreamError>
    where
        T: cpal::SizedSample + cpal::FromSample<f32>,
    {
        let feed = self.feed.clone();
        let failed = self.failed.clone();
        dev.build_output_stream(
            config,
            move |buf: &mut [T], _| {
                let mut f = feed.lock().unwrap_or_else(PoisonError::into_inner);
                for frame in buf.chunks_mut(channels) {
                    let v = if f.playing { next(&mut f) } else { 0.0 };
                    frame.fill(T::from_sample(v));
                }
            },
            move |_| failed.store(true, Ordering::SeqCst),
            None,
        )
    }
}

/// The next output sample (linear interpolation), advancing; stops at the end.
fn next(f: &mut Feed) -> f32 {
    let i = f.pos.max(0.0) as usize;
    let (Some(&a), b) = (f.samples.get(i), f.samples.get(i + 1).copied()) else {
        f.playing = false;
        return 0.0;
    };
    let t = (f.pos - i as f64) as f32;
    f.pos += f.step;
    a + (b.unwrap_or(a) - a) * t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resamples_and_stops_at_the_end() {
        let mut f = Feed { samples: vec![0.0, 1.0, 0.0], pos: 0.0, step: 0.5, playing: true };
        let out: Vec<f32> = (0..8).map(|_| next(&mut f)).collect();
        assert_eq!(&out[..5], &[0.0, 0.5, 1.0, 0.5, 0.0]);
        assert!(!f.playing);
        assert!(out[5..].iter().all(|&v| v == 0.0));
        let mut empty = Feed { playing: true, step: 1.0, ..Feed::default() };
        assert_eq!(next(&mut empty), 0.0);
        assert!(!empty.playing);
    }
}
