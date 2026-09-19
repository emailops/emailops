//! Minimal GGUF metadata reader.
//!
//! A GGUF file starts with a self-describing key/value header (architecture,
//! chat template, rope parameters, …) followed by the tensor data. Everything
//! this module needs — `general.architecture` — sits in that header, a few
//! hundred bytes in, so we read it with plain `std::io` instead of loading the
//! model: the answer is needed before (and independently of) any inference, and
//! on paths that must not pay for a multi-GB mmap.
//!
//! Like `gpu_plan`, this module is deliberately free of llama.cpp types so it
//! compiles and is tested in `--no-default-features` builds.
//!
//! Spec: <https://github.com/ggml-org/ggml/blob/master/docs/gguf.md>

use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

/// File magic every GGUF starts with.
const MAGIC: [u8; 4] = *b"GGUF";

/// Header versions this reader understands. v1 sized its lengths as `u32`
/// (and predates every model EmailOps can run), so it is rejected rather than
/// mis-parsed.
const SUPPORTED_VERSIONS: std::ops::RangeInclusive<u32> = 2..=3;

/// Upper bound on a single header string (key or value). Real keys are tens of
/// bytes and the longest value — the chat template — is tens of kilobytes; this
/// only exists so a corrupt length field cannot ask for a gigabyte allocation.
const MAX_STRING_BYTES: u64 = 8 * 1024 * 1024;

/// Upper bound on header entries walked before giving up. Real GGUFs carry a
/// few dozen; the cap keeps a corrupt count from spinning.
const MAX_KV_ENTRIES: u64 = 4096;

/// How deep nested arrays may go before we stop descending. Arrays of arrays
/// are legal but unused in practice.
const MAX_ARRAY_DEPTH: u32 = 4;

// GGUF value type tags.
const TY_UINT8: u32 = 0;
const TY_INT8: u32 = 1;
const TY_UINT16: u32 = 2;
const TY_INT16: u32 = 3;
const TY_UINT32: u32 = 4;
const TY_INT32: u32 = 5;
const TY_FLOAT32: u32 = 6;
const TY_BOOL: u32 = 7;
const TY_STRING: u32 = 8;
const TY_ARRAY: u32 = 9;
const TY_UINT64: u32 = 10;
const TY_INT64: u32 = 11;
const TY_FLOAT64: u32 = 12;

/// The metadata key naming the model family (`"qwen3"`, `"gemma3"`, `"llama"`,
/// …). Written by the converter from the source config, so it survives any
/// renaming of the file on disk.
pub const KEY_ARCHITECTURE: &str = "general.architecture";

/// Read `general.architecture` from a GGUF on disk.
///
/// `None` when the file is missing, unreadable, not a GGUF, or written in a
/// header version/shape this reader does not understand — callers treat that as
/// "unknown model family" and fall back to their own heuristics rather than
/// failing the inference.
pub fn read_architecture(path: &Path) -> Option<String> {
    read_metadata_string(path, KEY_ARCHITECTURE)
}

/// Read one string-valued metadata entry from a GGUF header.
///
/// Returns `None` for every failure mode (bad magic, unsupported version,
/// truncated file, key absent, key present but not a string) — this is a
/// best-effort probe, never an error path.
pub fn read_metadata_string(path: &Path, key: &str) -> Option<String> {
    let mut reader = BufReader::new(File::open(path).ok()?);

    let mut magic = [0u8; 4];
    reader.read_exact(&mut magic).ok()?;
    if magic != MAGIC {
        return None;
    }

    let version = read_u32(&mut reader)?;
    if !SUPPORTED_VERSIONS.contains(&version) {
        return None;
    }

    let _tensor_count = read_u64(&mut reader)?;
    let kv_count = read_u64(&mut reader)?;

    for _ in 0..kv_count.min(MAX_KV_ENTRIES) {
        let entry_key = read_string(&mut reader)?;
        let value_type = read_u32(&mut reader)?;
        if entry_key == key {
            return if value_type == TY_STRING {
                read_string(&mut reader)
            } else {
                None
            };
        }
        skip_value(&mut reader, value_type, 0)?;
    }

    None
}

fn read_u32(reader: &mut BufReader<File>) -> Option<u32> {
    let mut buf = [0u8; 4];
    reader.read_exact(&mut buf).ok()?;
    Some(u32::from_le_bytes(buf))
}

fn read_u64(reader: &mut BufReader<File>) -> Option<u64> {
    let mut buf = [0u8; 8];
    reader.read_exact(&mut buf).ok()?;
    Some(u64::from_le_bytes(buf))
}

/// A GGUF string: `u64` byte length followed by (non-NUL-terminated) UTF-8.
fn read_string(reader: &mut BufReader<File>) -> Option<String> {
    let len = read_u64(reader)?;
    if len > MAX_STRING_BYTES {
        return None;
    }
    let mut buf = vec![0u8; len as usize];
    reader.read_exact(&mut buf).ok()?;
    String::from_utf8(buf).ok()
}

/// Byte width of a fixed-size value type, or `None` for the variable-length
/// ones (string, array) and for unknown tags.
fn scalar_size(value_type: u32) -> Option<u64> {
    match value_type {
        TY_UINT8 | TY_INT8 | TY_BOOL => Some(1),
        TY_UINT16 | TY_INT16 => Some(2),
        TY_UINT32 | TY_INT32 | TY_FLOAT32 => Some(4),
        TY_UINT64 | TY_INT64 | TY_FLOAT64 => Some(8),
        _ => None,
    }
}

/// Advance past a value we don't care about. `None` on anything malformed so
/// the caller stops walking rather than reading garbage as a key.
fn skip_value(reader: &mut BufReader<File>, value_type: u32, depth: u32) -> Option<()> {
    match value_type {
        TY_STRING => {
            let len = read_u64(reader)?;
            skip_bytes(reader, len)
        }
        TY_ARRAY => {
            if depth >= MAX_ARRAY_DEPTH {
                return None;
            }
            let elem_type = read_u32(reader)?;
            let count = read_u64(reader)?;
            if let Some(size) = scalar_size(elem_type) {
                return skip_bytes(reader, count.checked_mul(size)?);
            }
            for _ in 0..count {
                skip_value(reader, elem_type, depth + 1)?;
            }
            Some(())
        }
        other => skip_bytes(reader, scalar_size(other)?),
    }
}

fn skip_bytes(reader: &mut BufReader<File>, count: u64) -> Option<()> {
    let offset = i64::try_from(count).ok()?;
    reader.seek_relative(offset).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// A GGUF string: u64 length + raw bytes.
    fn gguf_string(value: &str) -> Vec<u8> {
        let mut out = (value.len() as u64).to_le_bytes().to_vec();
        out.extend_from_slice(value.as_bytes());
        out
    }

    fn kv_string(key: &str, value: &str) -> Vec<u8> {
        let mut out = gguf_string(key);
        out.extend_from_slice(&TY_STRING.to_le_bytes());
        out.extend_from_slice(&gguf_string(value));
        out
    }

    fn kv_u32(key: &str, value: u32) -> Vec<u8> {
        let mut out = gguf_string(key);
        out.extend_from_slice(&TY_UINT32.to_le_bytes());
        out.extend_from_slice(&value.to_le_bytes());
        out
    }

    fn kv_bool(key: &str, value: bool) -> Vec<u8> {
        let mut out = gguf_string(key);
        out.extend_from_slice(&TY_BOOL.to_le_bytes());
        out.push(u8::from(value));
        out
    }

    fn kv_string_array(key: &str, values: &[&str]) -> Vec<u8> {
        let mut out = gguf_string(key);
        out.extend_from_slice(&TY_ARRAY.to_le_bytes());
        out.extend_from_slice(&TY_STRING.to_le_bytes());
        out.extend_from_slice(&(values.len() as u64).to_le_bytes());
        for value in values {
            out.extend_from_slice(&gguf_string(value));
        }
        out
    }

    fn kv_f32_array(key: &str, values: &[f32]) -> Vec<u8> {
        let mut out = gguf_string(key);
        out.extend_from_slice(&TY_ARRAY.to_le_bytes());
        out.extend_from_slice(&TY_FLOAT32.to_le_bytes());
        out.extend_from_slice(&(values.len() as u64).to_le_bytes());
        for value in values {
            out.extend_from_slice(&value.to_le_bytes());
        }
        out
    }

    /// Assemble a GGUF header (no tensor data — this reader never reaches it).
    fn gguf_header(version: u32, kvs: &[Vec<u8>]) -> Vec<u8> {
        let mut out = MAGIC.to_vec();
        out.extend_from_slice(&version.to_le_bytes());
        out.extend_from_slice(&0u64.to_le_bytes()); // tensor_count
        out.extend_from_slice(&(kvs.len() as u64).to_le_bytes());
        for kv in kvs {
            out.extend_from_slice(kv);
        }
        out
    }

    fn write_fixture(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn reads_architecture_regardless_of_file_name() {
        let tmp = tempfile::tempdir().unwrap();
        // The file name says nothing about the family — the header does.
        let path = write_fixture(
            tmp.path(),
            "my-favourite-model-q4_k_m.gguf",
            &gguf_header(3, &[kv_string(KEY_ARCHITECTURE, "qwen3")]),
        );
        assert_eq!(read_architecture(&path).as_deref(), Some("qwen3"));
    }

    #[test]
    fn reads_architecture_after_other_entry_types() {
        let tmp = tempfile::tempdir().unwrap();
        // Real GGUFs put tokenizer arrays and numeric hyperparameters before
        // (and after) the key we want — every value type must be skippable.
        let path = write_fixture(
            tmp.path(),
            "model.gguf",
            &gguf_header(
                3,
                &[
                    kv_u32("qwen3.block_count", 48),
                    kv_bool("tokenizer.ggml.add_bos_token", false),
                    kv_string_array("tokenizer.ggml.tokens", &["<|im_start|>", "hola", "adiós"]),
                    kv_f32_array("qwen3.attention.scale", &[0.5, 0.25]),
                    kv_string("general.name", "Some Model 4B"),
                    kv_string(KEY_ARCHITECTURE, "qwen3moe"),
                ],
            ),
        );
        assert_eq!(read_architecture(&path).as_deref(), Some("qwen3moe"));
    }

    #[test]
    fn reads_architecture_from_v2_header() {
        let tmp = tempfile::tempdir().unwrap();
        let path = write_fixture(
            tmp.path(),
            "old.gguf",
            &gguf_header(2, &[kv_string(KEY_ARCHITECTURE, "llama")]),
        );
        assert_eq!(read_architecture(&path).as_deref(), Some("llama"));
    }

    #[test]
    fn unreadable_or_foreign_files_yield_none() {
        let tmp = tempfile::tempdir().unwrap();

        // Missing file.
        assert_eq!(read_architecture(&tmp.path().join("absent.gguf")), None);

        // Not a GGUF at all.
        let not_gguf = write_fixture(tmp.path(), "notes.txt", b"just some bytes");
        assert_eq!(read_architecture(&not_gguf), None);

        // Header version this reader does not parse.
        let v1 = write_fixture(
            tmp.path(),
            "v1.gguf",
            &gguf_header(1, &[kv_string(KEY_ARCHITECTURE, "qwen3")]),
        );
        assert_eq!(read_architecture(&v1), None);

        // Truncated mid-value.
        let full = gguf_header(3, &[kv_string(KEY_ARCHITECTURE, "qwen3")]);
        let truncated = write_fixture(tmp.path(), "cut.gguf", &full[..full.len() - 3]);
        assert_eq!(read_architecture(&truncated), None);

        // Key absent from an otherwise valid header.
        let no_key = write_fixture(
            tmp.path(),
            "nokey.gguf",
            &gguf_header(3, &[kv_string("general.name", "x")]),
        );
        assert_eq!(read_architecture(&no_key), None);

        // Key present but not a string.
        let wrong_type = write_fixture(
            tmp.path(),
            "wrongtype.gguf",
            &gguf_header(3, &[kv_u32(KEY_ARCHITECTURE, 7)]),
        );
        assert_eq!(read_architecture(&wrong_type), None);
    }

    #[test]
    fn corrupt_lengths_do_not_allocate_or_spin() {
        let tmp = tempfile::tempdir().unwrap();

        // A key length far beyond the file (and beyond MAX_STRING_BYTES).
        let mut absurd = MAGIC.to_vec();
        absurd.extend_from_slice(&3u32.to_le_bytes());
        absurd.extend_from_slice(&0u64.to_le_bytes());
        absurd.extend_from_slice(&1u64.to_le_bytes());
        absurd.extend_from_slice(&u64::MAX.to_le_bytes());
        let path = write_fixture(tmp.path(), "absurd.gguf", &absurd);
        assert_eq!(read_architecture(&path), None);

        // A kv_count far larger than the entries actually present: the walk
        // ends when the reads run out, not after MAX_KV_ENTRIES of garbage.
        let mut over_count = MAGIC.to_vec();
        over_count.extend_from_slice(&3u32.to_le_bytes());
        over_count.extend_from_slice(&0u64.to_le_bytes());
        over_count.extend_from_slice(&u64::MAX.to_le_bytes());
        over_count.extend_from_slice(&kv_string("general.name", "x"));
        let path = write_fixture(tmp.path(), "overcount.gguf", &over_count);
        assert_eq!(read_architecture(&path), None);
    }
}
