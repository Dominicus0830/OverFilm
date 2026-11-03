//! Reading the broadcast HUD's text (feature `ocr`): team names over the player bars and the
//! players' names on the name plates, with the [ocrs](https://github.com/robertknight/ocrs)
//! models (Latin script, pure Rust on rten). What it reads is matched against the glossary's
//! roster by `overfilm_vision::lineup`; [`roster`] builds that roster from the built-in pack.
//!
//! The text is small (player names are 8 pixels tall at 720p), so each part is enlarged
//! [`SCALE`] times before it is read.

use std::path::Path;

use filmcraft_speech::models::ModelFile;
use overfilm_asr::catalogue::Model;
use overfilm_vision::lineup::Word;
use overfilm_vision::{NRect, Rgb};

/// The two ocrs models (MIT OR Apache-2.0 code; models by Robert Knight, trained on the HierText
/// dataset), pinned by SHA-256.
pub static MODELS: Model = Model {
    id: "ocrs",
    name: "ocrs 글자 인식",
    source: "https://github.com/robertknight/ocrs",
    license: "CC-BY-SA-4.0 (models), MIT OR Apache-2.0 (code)",
    files: &[
        ModelFile {
            name: "text-detection.onnx",
            url: "https://ocrs-models.s3-accelerate.amazonaws.com/text-detection.onnx",
            sha256: "a917b23dbd9524b465df7e922641b3ff2981623df4ded5a0234004ef2fee7cfe",
            size: 2_499_479,
        },
        ModelFile {
            name: "text-recognition.onnx",
            url: "https://ocrs-models.s3-accelerate.amazonaws.com/text-recognition.onnx",
            sha256: "86c145c2edb96c8caed5b1ebb8f44d706408922211451c309c625157dd6061c5",
            size: 9_713_177,
        },
    ],
};

/// How much a part of the frame is enlarged before it is read.
pub const SCALE: f32 = 2.5;
/// Largest picture handed to the models (pixels across, after enlarging).
const MAX_SIDE: usize = 2400;

/// Download the models if they are missing; `progress(done, total)` returns false to cancel.
pub fn ensure(models_dir: &Path, progress: &mut dyn FnMut(u64, u64) -> bool) -> Result<(), String> {
    if MODELS.installed(models_dir) {
        return Ok(());
    }
    overfilm_asr::catalogue::download(models_dir, &MODELS, &mut |d, t, _| progress(d, t)).map_err(|e| format!("글자 인식 모델 다운로드 실패: {e}"))
}

/// A loaded text reader.
pub struct Ocr {
    engine: ocrs::OcrEngine,
}

impl Ocr {
    /// Load the models from `models_dir` (see [`ensure`]).
    pub fn load(models_dir: &Path) -> Result<Ocr, String> {
        let dir = MODELS.dir(models_dir);
        let load = |name: &str| rten::Model::load_file(dir.join(name)).map_err(|e| format!("{name}: {e}"));
        let engine = ocrs::OcrEngine::new(ocrs::OcrEngineParams {
            detection_model: Some(load("text-detection.onnx")?),
            recognition_model: Some(load("text-recognition.onnx")?),
            ..Default::default()
        })
        .map_err(|e| e.to_string())?;
        Ok(Ocr { engine })
    }

    /// The words inside `rect` of an RGBA frame, with their centres and widths in frame-relative
    /// coordinates.
    pub fn read(&self, rgba: &[u8], width: usize, height: usize, rect: NRect) -> Vec<(Word, f32)> {
        let Some(crop) = Rgb::crop_rgba(rgba, width, height, rect, 2048) else { return Vec::new() };
        let Some(r) = rect.clamped() else { return Vec::new() };
        let scale = SCALE.min(MAX_SIDE as f32 / crop.width.max(1) as f32).max(1.0);
        let big = enlarge(&crop, scale);
        let (Ok(w), Ok(h)) = (u32::try_from(big.width), u32::try_from(big.height)) else { return Vec::new() };
        let Ok(src) = ocrs::ImageSource::from_bytes(&big.px, (w, h)) else { return Vec::new() };
        let Ok(input) = self.engine.prepare_input(src) else { return Vec::new() };
        let Ok(words) = self.engine.detect_words(&input) else { return Vec::new() };
        let lines = self.engine.find_text_lines(&input, &words);
        let Ok(texts) = self.engine.recognize_text(&input, &lines) else { return Vec::new() };
        let (bw, bh) = (big.width.max(1) as f32, big.height.max(1) as f32);
        let mut out = Vec::new();
        for line in texts.iter().flatten() {
            for word in line.words() {
                use ocrs::TextItem;
                let b = word.bounding_rect();
                let cx = (b.left() + b.right()) as f32 / 2.0 / bw;
                let cy = (b.top() + b.bottom()) as f32 / 2.0 / bh;
                let ww = (b.right() - b.left()).max(0) as f32 / bw;
                out.push((Word { x: r.x + cx * r.w, y: r.y + cy * r.h, text: word.to_string() }, ww * r.w));
            }
        }
        out
    }
}

/// `src` enlarged `scale` times (bilinear).
fn enlarge(src: &Rgb, scale: f32) -> Rgb {
    let w = ((src.width as f32 * scale) as usize).clamp(1, MAX_SIDE);
    let h = ((src.height as f32 * scale) as usize).clamp(1, MAX_SIDE);
    let mut px = Vec::with_capacity(w * h * 3);
    let (sx, sy) = (src.width as f32 / w as f32, src.height as f32 / h as f32);
    for y in 0..h {
        let fy = ((y as f32 + 0.5) * sy - 0.5).max(0.0);
        let (y0, ty) = (fy as usize, fy.fract());
        for x in 0..w {
            let fx = ((x as f32 + 0.5) * sx - 0.5).max(0.0);
            let (x0, tx) = (fx as usize, fx.fract());
            let (a, b, c, d) = (
                src.at(x0, y0),
                src.at((x0 + 1).min(src.width - 1), y0),
                src.at(x0, (y0 + 1).min(src.height - 1)),
                src.at((x0 + 1).min(src.width - 1), (y0 + 1).min(src.height - 1)),
            );
            for ch in 0..3 {
                let top = a[ch] as f32 * (1.0 - tx) + b[ch] as f32 * tx;
                let bot = c[ch] as f32 * (1.0 - tx) + d[ch] as f32 * tx;
                px.push((top * (1.0 - ty) + bot * ty).round().clamp(0.0, 255.0) as u8);
            }
        }
    }
    Rgb { width: w, height: h, px }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enlarging_keeps_corners_and_size() {
        let src = Rgb::from_rgb(vec![0, 0, 0, 255, 255, 255, 0, 0, 0, 255, 255, 255], 2, 2).unwrap();
        let big = enlarge(&src, 2.5);
        assert_eq!((big.width, big.height), (5, 5));
        assert_eq!(big.at(0, 0), [0, 0, 0]);
        assert_eq!(big.at(4, 4), [255, 255, 255]);
        // a huge scale is capped
        let huge = enlarge(&src, 1e9);
        assert!(huge.width <= MAX_SIDE && huge.height <= MAX_SIDE);
    }

    #[test]
    fn pinned_models() {
        assert_eq!(MODELS.files.len(), 2);
        assert!(MODELS.files.iter().all(|f| f.sha256.len() == 64 && f.size > 0 && f.url.starts_with("https://")));
    }
}
