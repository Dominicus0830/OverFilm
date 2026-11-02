//! The 8-bit decoder weights saved next to the model, so later runs skip reading the 16-bit
//! checkpoint for them and quantizing it again (about two thirds of loading the 1.7B model).
//!
//! The file is tied to the checkpoint it came from: its key lists every shard's name, size and
//! modification time, and any difference (a re-download, a new model version, another quantizer)
//! makes it stale, so it is rebuilt. It is written to a temporary name and renamed when complete,
//! so a run stopped while writing leaves no half file behind. Any problem reading it only means
//! the weights are quantized again from the checkpoint.
//!
//! Format: magic, key length (u64 LE) and key, tensor count (u64), then per tensor: name length
//! (u32) and name, rows and columns (u64 each), the scales (f32 LE, one per 32 values) and the
//! values (i8).

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::qmm::Q8;

const MAGIC: &[u8; 12] = b"OVERFILM-Q8\n";
/// Bumped whenever the quantization or the layout changes.
const VERSION: u32 = 1;
const FILE: &str = "overfilm-q8.cache";
const MAX_KEY: u64 = 1 << 20;
const MAX_NAME: u32 = 4096;
const MAX_TENSORS: u64 = 100_000;

/// Where the cache of the model in `dir` lives.
pub fn path(dir: &Path) -> PathBuf {
    dir.join(FILE)
}

/// What the cache must match: the format version and each shard's name, size and time.
pub fn key(shards: &[PathBuf]) -> String {
    let mut k = format!("v{VERSION}");
    for p in shards {
        let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let (len, modified) = std::fs::metadata(p)
            .map(|m| {
                let t = m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_nanos()).unwrap_or(0);
                (m.len(), t)
            })
            .unwrap_or((0, 0));
        k.push_str(&format!("|{name}:{len}:{modified}"));
    }
    k
}

fn read_u64(r: &mut impl Read) -> Option<u64> {
    let mut b = [0u8; 8];
    r.read_exact(&mut b).ok()?;
    Some(u64::from_le_bytes(b))
}

/// The cached weights of `dir` when the cache exists and matches `key`; `None` otherwise.
pub fn read(dir: &Path, key: &str) -> Option<HashMap<String, Arc<Q8>>> {
    let f = File::open(path(dir)).ok()?;
    let mut left = f.metadata().ok()?.len();
    let mut r = BufReader::with_capacity(1 << 20, f);
    let mut take = |n: u64| -> Option<()> {
        left = left.checked_sub(n)?;
        Some(())
    };
    let mut magic = [0u8; 12];
    take(12)?;
    r.read_exact(&mut magic).ok()?;
    if &magic != MAGIC {
        return None;
    }
    take(8)?;
    let klen = read_u64(&mut r)?;
    if klen > MAX_KEY {
        return None;
    }
    take(klen)?;
    let mut kb = vec![0u8; usize::try_from(klen).ok()?];
    r.read_exact(&mut kb).ok()?;
    if kb != key.as_bytes() {
        return None;
    }
    take(8)?;
    let count = read_u64(&mut r)?;
    if count > MAX_TENSORS {
        return None;
    }
    let mut out = HashMap::new();
    for _ in 0..count {
        take(4)?;
        let mut nb = [0u8; 4];
        r.read_exact(&mut nb).ok()?;
        let nlen = u32::from_le_bytes(nb);
        if nlen > MAX_NAME {
            return None;
        }
        take(u64::from(nlen))?;
        let mut name = vec![0u8; usize::try_from(nlen).ok()?];
        r.read_exact(&mut name).ok()?;
        let name = String::from_utf8(name).ok()?;
        take(16)?;
        let n = usize::try_from(read_u64(&mut r)?).ok()?;
        let k = usize::try_from(read_u64(&mut r)?).ok()?;
        let len = n.checked_mul(k)?;
        let blocks = len / 32;
        // both parts must be in the file before anything that size is allocated
        take(u64::try_from(blocks.checked_mul(4)?.checked_add(len)?).ok()?)?;
        let mut d = vec![0f32; blocks];
        r.read_exact(bytemuck::cast_slice_mut(&mut d)).ok()?;
        for x in &mut d {
            *x = f32::from_bits(u32::from_le(x.to_bits()));
        }
        let mut qs = vec![0i8; len];
        r.read_exact(bytemuck::cast_slice_mut(&mut qs)).ok()?;
        out.insert(name, Arc::new(Q8::from_parts(n, k, d, qs)?));
    }
    Some(out)
}

/// Save `weights` as the cache of `dir` for `key`.
pub fn write(dir: &Path, key: &str, weights: &HashMap<String, Arc<Q8>>) -> std::io::Result<()> {
    let tmp = dir.join(format!("{FILE}.partial"));
    {
        let mut w = BufWriter::with_capacity(1 << 20, File::create(&tmp)?);
        w.write_all(MAGIC)?;
        w.write_all(&(key.len() as u64).to_le_bytes())?;
        w.write_all(key.as_bytes())?;
        w.write_all(&(weights.len() as u64).to_le_bytes())?;
        let mut names: Vec<&String> = weights.keys().collect();
        names.sort();
        for name in names {
            let Some(q) = weights.get(name) else { continue };
            let (n, k) = q.dims();
            w.write_all(&u32::try_from(name.len()).unwrap_or(u32::MAX).to_le_bytes())?;
            w.write_all(name.as_bytes())?;
            w.write_all(&(n as u64).to_le_bytes())?;
            w.write_all(&(k as u64).to_le_bytes())?;
            if cfg!(target_endian = "little") {
                w.write_all(bytemuck::cast_slice(q.scales()))?;
            } else {
                for s in q.scales() {
                    w.write_all(&s.to_le_bytes())?;
                }
            }
            w.write_all(bytemuck::cast_slice(q.values()))?;
        }
        w.flush()?;
        w.into_inner().map_err(|e| e.into_error())?.sync_all()?;
    }
    std::fs::rename(&tmp, path(dir))
}

/// [`write`] on a background thread (loading goes on; the weights are shared, not copied).
/// Failures are only reported when tracing: the next run quantizes again.
pub fn write_later(dir: &Path, key: String, weights: HashMap<String, Arc<Q8>>) {
    let dir = dir.to_path_buf();
    let _ = std::thread::Builder::new().name("q8-cache".into()).spawn(move || {
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| write(&dir, &key, &weights)));
        if std::env::var_os("OVERFILM_ASR_TRACE").is_some() {
            eprintln!("q8 cache {}: {:?}", path(&dir).display(), r.as_ref().map(|r| r.as_ref().map_err(ToString::to_string)));
        }
        if !matches!(r, Ok(Ok(()))) {
            let _ = std::fs::remove_file(dir.join(format!("{FILE}.partial")));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use candle_core::{Device, Tensor};

    fn weights() -> HashMap<String, Arc<Q8>> {
        let mut m = HashMap::new();
        for (i, (n, k)) in [(3, 64), (5, 32)].into_iter().enumerate() {
            let v: Vec<f32> = (0..n * k).map(|j| ((j * 7 + i) % 13) as f32 - 6.0).collect();
            let t = Tensor::from_vec(v, (n, k), &Device::Cpu).unwrap();
            m.insert(format!("w{i}"), Arc::new(Q8::quantize(&t).unwrap()));
        }
        m
    }

    #[test]
    fn round_trips_and_checks_its_key() {
        let d = tempfile::tempdir().unwrap();
        let w = weights();
        write(d.path(), "k1", &w).unwrap();
        let back = read(d.path(), "k1").unwrap();
        assert_eq!(back.len(), 2);
        for (name, q) in &w {
            let b = &back[name];
            assert_eq!(b.dims(), q.dims());
            assert_eq!(b.scales(), q.scales());
            assert_eq!(b.values(), q.values());
        }
        assert!(read(d.path(), "k2").is_none(), "another checkpoint: stale");
        assert!(!d.path().join(format!("{FILE}.partial")).exists());
    }

    #[test]
    fn damaged_files_are_ignored_not_trusted() {
        let d = tempfile::tempdir().unwrap();
        write(d.path(), "k", &weights()).unwrap();
        let full = std::fs::read(path(d.path())).unwrap();
        // every cut is rejected
        for cut in [0, 5, 12, 20, 30, full.len() / 2, full.len() - 1] {
            std::fs::write(path(d.path()), &full[..cut]).unwrap();
            assert!(read(d.path(), "k").is_none(), "cut at {cut}");
        }
        // a hostile size is rejected before anything that big is allocated
        let mut bad = full.clone();
        let at = 12 + 8 + 1 + 8 + 4 + 2;
        bad[at..at + 8].copy_from_slice(&u64::MAX.to_le_bytes());
        std::fs::write(path(d.path()), &bad).unwrap();
        assert!(read(d.path(), "k").is_none());
        assert!(read(&d.path().join("missing"), "k").is_none());
    }

    #[test]
    fn key_follows_the_shards() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("model.safetensors");
        std::fs::write(&p, b"abc").unwrap();
        let k1 = key(std::slice::from_ref(&p));
        assert!(k1.contains("model.safetensors:3:"));
        std::fs::write(&p, b"abcd").unwrap();
        assert_ne!(key(std::slice::from_ref(&p)), k1);
    }
}
