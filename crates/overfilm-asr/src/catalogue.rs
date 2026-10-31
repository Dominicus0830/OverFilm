//! The two models, pinned to a Hugging Face revision with SHA-256 per file, and their download
//! (filmcraft-speech's verified downloader: `.part` file, checksum, rename).
//!
//! Qwen3-ASR-1.7B and Qwen3-ForcedAligner-0.6B: Apache-2.0, Qwen team (Alibaba Cloud).

use std::path::{Path, PathBuf};

use filmcraft_speech::models::ModelFile;

/// One downloadable model.
pub struct Model {
    pub id: &'static str,
    pub name: &'static str,
    pub source: &'static str,
    pub license: &'static str,
    pub files: &'static [ModelFile],
}

impl Model {
    pub fn size(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }

    pub fn dir(&self, models_dir: &Path) -> PathBuf {
        models_dir.join(self.id)
    }

    /// Every file present with the expected size (checksums are checked when downloading).
    pub fn installed(&self, models_dir: &Path) -> bool {
        let d = self.dir(models_dir);
        self.files.iter().all(|f| std::fs::metadata(d.join(f.name)).is_ok_and(|m| m.len() == f.size))
    }

    /// Bytes still to download.
    pub fn missing_bytes(&self, models_dir: &Path) -> u64 {
        let d = self.dir(models_dir);
        self.files.iter().filter(|f| !std::fs::metadata(d.join(f.name)).is_ok_and(|m| m.len() == f.size)).map(|f| f.size).sum()
    }
}

macro_rules! hf {
    ($repo:literal, $rev:literal, $name:literal, $sha:literal, $size:expr) => {
        ModelFile { name: $name, url: concat!("https://huggingface.co/Qwen/", $repo, "/resolve/", $rev, "/", $name), sha256: $sha, size: $size }
    };
}

const VOCAB_SHA: &str = "ca10d7e9fb3ed18575dd1e277a2579c16d108e32f27439684afa0e10b1440910";
const MERGES_SHA: &str = "8831e4f1a044471340f7c0a83d7bd71306a5b867e95fd870f74d0c5308a904d5";

pub static ASR: Model = Model {
    id: "qwen3-asr-1.7b",
    name: "Qwen3-ASR 1.7B",
    source: "https://huggingface.co/Qwen/Qwen3-ASR-1.7B",
    license: "Apache-2.0",
    files: &[
        hf!(
            "Qwen3-ASR-1.7B",
            "7278e1e70fe206f11671096ffdd38061171dd6e5",
            "config.json",
            "2e74a751548b8ad7d7526d29365ad8144c345d8b412b1152d25dc6698452712f",
            6194
        ),
        hf!(
            "Qwen3-ASR-1.7B",
            "7278e1e70fe206f11671096ffdd38061171dd6e5",
            "tokenizer_config.json",
            "4942d005604266809309cabc9f4e9cb89ce855d59b14681fdc0e1cc62ea26c4c",
            12487
        ),
        ModelFile {
            name: "vocab.json",
            url: "https://huggingface.co/Qwen/Qwen3-ASR-1.7B/resolve/7278e1e70fe206f11671096ffdd38061171dd6e5/vocab.json",
            sha256: VOCAB_SHA,
            size: 2_776_833,
        },
        ModelFile {
            name: "merges.txt",
            url: "https://huggingface.co/Qwen/Qwen3-ASR-1.7B/resolve/7278e1e70fe206f11671096ffdd38061171dd6e5/merges.txt",
            sha256: MERGES_SHA,
            size: 1_671_853,
        },
        hf!(
            "Qwen3-ASR-1.7B",
            "7278e1e70fe206f11671096ffdd38061171dd6e5",
            "model-00001-of-00002.safetensors",
            "a4cd1f1a04d90b757dc7f7dd26254e69a013b19e80efe590a83c6a3bde8608d6",
            4_220_320_824
        ),
        hf!(
            "Qwen3-ASR-1.7B",
            "7278e1e70fe206f11671096ffdd38061171dd6e5",
            "model-00002-of-00002.safetensors",
            "6e0b9d9e09e2e0238e7ef3cc8a484ab387e91b90f1900bedf88bc92d7929ccfc",
            478_200_688
        ),
    ],
};

pub static ALIGNER: Model = Model {
    id: "qwen3-forcedaligner-0.6b",
    name: "Qwen3-ForcedAligner 0.6B",
    source: "https://huggingface.co/Qwen/Qwen3-ForcedAligner-0.6B",
    license: "Apache-2.0",
    files: &[
        hf!(
            "Qwen3-ForcedAligner-0.6B",
            "c7cbfc2048c462b0d63a45797104fc9db3ad62b7",
            "config.json",
            "d616c65d46c4b90bdc651b0a0963ea932732241140f337f9bb6b0335a9c8ef09",
            5982
        ),
        hf!(
            "Qwen3-ForcedAligner-0.6B",
            "c7cbfc2048c462b0d63a45797104fc9db3ad62b7",
            "tokenizer_config.json",
            "3ab80063f8511deb9566e6ad438d17b7a6277fcffd52d92854112f19d36bd81c",
            12666
        ),
        ModelFile {
            name: "vocab.json",
            url: "https://huggingface.co/Qwen/Qwen3-ForcedAligner-0.6B/resolve/c7cbfc2048c462b0d63a45797104fc9db3ad62b7/vocab.json",
            sha256: VOCAB_SHA,
            size: 2_776_833,
        },
        ModelFile {
            name: "merges.txt",
            url: "https://huggingface.co/Qwen/Qwen3-ForcedAligner-0.6B/resolve/c7cbfc2048c462b0d63a45797104fc9db3ad62b7/merges.txt",
            sha256: MERGES_SHA,
            size: 1_671_853,
        },
        hf!(
            "Qwen3-ForcedAligner-0.6B",
            "c7cbfc2048c462b0d63a45797104fc9db3ad62b7",
            "model.safetensors",
            "47831d0e82f96b20e9034dba01a075ee06436654719f6a68289e49f1b65ce0e7",
            1_835_544_544
        ),
    ],
};

/// The speaker network (who spoke when): WeSpeaker ResNet34 trained on VoxCeleb, as published by
/// pyannote. CC-BY-4.0 (attribution in `OVERFILM.md` and the program's README).
pub static SPEAKER: Model = Model {
    id: "wespeaker-voxceleb-resnet34-lm",
    name: "WeSpeaker ResNet34 (화자 구별)",
    source: "https://huggingface.co/pyannote/wespeaker-voxceleb-resnet34-LM",
    license: "CC-BY-4.0",
    files: &[ModelFile {
        name: "pytorch_model.bin",
        url: "https://huggingface.co/pyannote/wespeaker-voxceleb-resnet34-LM/resolve/837717ddb9ff5507820346191109dc79c958d614/pytorch_model.bin",
        sha256: "366edf44f4c80889a3eb7a9d7bdf02c4aede3127f7dd15e274dcdb826b143c56",
        size: 26_645_418,
    }],
};

/// Overlapped-speech model: pyannote segmentation-3.0 (MIT). The original repository needs a
/// Hugging Face sign-in; this is the ONNX export published openly by onnx-community (same
/// weights, checked against the PyTorch-exported graph with onnxruntime).
pub static SEGMENT: Model = Model {
    id: "pyannote-segmentation-3.0",
    name: "pyannote segmentation 3.0 (겹친 말)",
    source: "https://huggingface.co/onnx-community/pyannote-segmentation-3.0",
    license: "MIT",
    files: &[ModelFile {
        name: "model.onnx",
        url: "https://huggingface.co/onnx-community/pyannote-segmentation-3.0/resolve/733a93b6473d019a773298e08cefa686894b1854/onnx/model.onnx",
        sha256: "057ee564753071c0b09b5b611648b50ac188d50846bff5f01e9f7bbf1591ea25",
        size: 5_986_908,
    }],
};

/// Every model, recogniser first.
pub fn all() -> [&'static Model; 4] {
    [&ASR, &ALIGNER, &SPEAKER, &SEGMENT]
}

/// Download whatever is missing of `m` into `models_dir/<id>/`; `progress(done, total, file)`
/// returns false to cancel.
#[cfg(feature = "download")]
pub fn download(models_dir: &Path, m: &Model, progress: &mut dyn FnMut(u64, u64, &str) -> bool) -> Result<(), crate::AsrError> {
    filmcraft_speech::models::download_files(&m.dir(models_dir), m.files, progress).map_err(|e| match e {
        filmcraft_speech::SpeechError::Cancelled => crate::AsrError::Cancelled,
        e => crate::AsrError::Download(e.to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pinned_and_complete() {
        for m in all() {
            assert!(m.files.iter().all(|f| f.url.starts_with("https://huggingface.co/") && f.url.ends_with(f.name) && f.sha256.len() == 64));
            if m.id == SPEAKER.id || m.id == SEGMENT.id {
                continue;
            }
            for need in ["config.json", "vocab.json", "merges.txt", "tokenizer_config.json"] {
                assert!(m.files.iter().any(|f| f.name == need), "{} lacks {need}", m.id);
            }
        }
        assert!(ASR.size() > 4_000_000_000);
        assert!(!ASR.installed(Path::new("/nonexistent")));
    }
}
