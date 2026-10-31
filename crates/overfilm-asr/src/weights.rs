//! Streaming safetensors reader: tensors are read one at a time from the file and converted to the
//! compute dtype, so loading a 4 GB bf16 shard never holds the whole file in memory next to its
//! f32 copy. Format: 8-byte little-endian header length, JSON header
//! (`name → {dtype, shape, data_offsets}`), then the data.

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use candle_core::{DType, Device, Tensor};

use crate::AsrError;

/// Largest header we accept (the real ones are under 100 KB).
const MAX_HEADER: u64 = 64 << 20;

fn dtype(s: &str) -> Option<DType> {
    Some(match s {
        "BF16" => DType::BF16,
        "F16" => DType::F16,
        "F32" => DType::F32,
        _ => return None,
    })
}

/// Load the tensors of `path` whose names pass `keep`, converted to `to`, into `out`.
pub fn load(path: &Path, to: DType, dev: &Device, keep: &dyn Fn(&str) -> bool, out: &mut HashMap<String, Tensor>) -> Result<(), AsrError> {
    each(path, to, dev, keep, &mut |name, t| {
        out.insert(name, t);
        Ok(())
    })
}

/// Hand each tensor of `path` whose name passes `keep`, converted to `to`, to `sink` as soon as it
/// is read (so the caller can quantize it before the next one is loaded).
pub fn each(
    path: &Path,
    to: DType,
    dev: &Device,
    keep: &dyn Fn(&str) -> bool,
    sink: &mut dyn FnMut(String, Tensor) -> Result<(), AsrError>,
) -> Result<(), AsrError> {
    let err = |e: &dyn std::fmt::Display| AsrError::Model(format!("{}: {e}", path.display()));
    let mut f = File::open(path).map_err(|e| err(&e))?;
    let file_len = f.metadata().map_err(|e| err(&e))?.len();
    let mut len8 = [0u8; 8];
    f.read_exact(&mut len8).map_err(|e| err(&e))?;
    let hlen = u64::from_le_bytes(len8);
    if hlen > MAX_HEADER || hlen.saturating_add(8) > file_len {
        return Err(err(&"bad safetensors header length"));
    }
    let mut hbuf = vec![0u8; usize::try_from(hlen).map_err(|e| err(&e))?];
    f.read_exact(&mut hbuf).map_err(|e| err(&e))?;
    let header: serde_json::Map<String, serde_json::Value> = serde_json::from_slice(&hbuf).map_err(|e| err(&e))?;
    let base = 8 + hlen;
    for (name, info) in &header {
        if name == "__metadata__" || !keep(name) {
            continue;
        }
        let dt = info.get("dtype").and_then(|v| v.as_str()).and_then(dtype).ok_or_else(|| err(&format!("{name}: unsupported dtype")))?;
        let shape: Vec<usize> = info
            .get("shape")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_u64().and_then(|n| usize::try_from(n).ok())).collect())
            .ok_or_else(|| err(&format!("{name}: no shape")))?;
        let offs: Vec<u64> = info.get("data_offsets").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|x| x.as_u64()).collect()).unwrap_or_default();
        let (Some(&start), Some(&end)) = (offs.first(), offs.get(1)) else {
            return Err(err(&format!("{name}: no data offsets")));
        };
        let elems = shape.iter().try_fold(1usize, |a, &d| a.checked_mul(d)).ok_or_else(|| err(&format!("{name}: shape overflow")))?;
        let bytes = elems.checked_mul(dt.size_in_bytes()).ok_or_else(|| err(&format!("{name}: size overflow")))?;
        if end < start || end - start != bytes as u64 || base.saturating_add(end) > file_len {
            return Err(err(&format!("{name}: data offsets out of range")));
        }
        let mut buf = vec![0u8; bytes];
        f.seek(SeekFrom::Start(base + start)).map_err(|e| err(&e))?;
        f.read_exact(&mut buf).map_err(|e| err(&e))?;
        let t = Tensor::from_raw_buffer(&buf, dt, &shape, dev).and_then(|t| t.to_dtype(to)).map_err(|e| err(&e))?;
        sink(name.clone(), t)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, header: &str, data: &[u8]) -> std::path::PathBuf {
        let p = dir.join("t.safetensors");
        let mut v = (header.len() as u64).to_le_bytes().to_vec();
        v.extend_from_slice(header.as_bytes());
        v.extend_from_slice(data);
        std::fs::write(&p, v).unwrap();
        p
    }

    #[test]
    fn reads_and_converts() {
        let d = tempfile::tempdir().unwrap();
        let data: Vec<u8> = [1.0f32, 2.0, 3.0, 4.0].iter().flat_map(|x| x.to_le_bytes()).collect();
        let p = write(d.path(), r#"{"w":{"dtype":"F32","shape":[2,2],"data_offsets":[0,16]},"skip":{"dtype":"F32","shape":[1],"data_offsets":[0,4]}}"#, &data);
        let mut m = HashMap::new();
        load(&p, DType::F32, &Device::Cpu, &|n| n != "skip", &mut m).unwrap();
        assert_eq!(m.len(), 1);
        assert_eq!(m["w"].to_vec2::<f32>().unwrap(), vec![vec![1.0, 2.0], vec![3.0, 4.0]]);
    }

    #[test]
    fn rejects_hostile_offsets() {
        let d = tempfile::tempdir().unwrap();
        let p = write(d.path(), r#"{"w":{"dtype":"F32","shape":[4000000000,4000000000],"data_offsets":[0,16]}}"#, &[0; 16]);
        assert!(load(&p, DType::F32, &Device::Cpu, &|_| true, &mut HashMap::new()).is_err());
        let p = write(d.path(), r#"{"w":{"dtype":"F32","shape":[4],"data_offsets":[0,999]}}"#, &[0; 16]);
        assert!(load(&p, DType::F32, &Device::Cpu, &|_| true, &mut HashMap::new()).is_err());
    }
}
