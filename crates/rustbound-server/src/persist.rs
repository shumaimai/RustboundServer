//! World block override persistence.
//!
//! Saves and loads the `World` block overrides map to/from disk under a
//! directory derived from `ServerConfig.level_name`. Uses a simple custom
//! binary format (no external serialization dependencies).
//!
//! ## Format (v1)
//!
//! ```text
//! Magic:   4 bytes  = "RBOV"
//! Version: u32 LE   = 1
//! Count:   u32 LE   = number of entries
//! Entries: Count * (i32 LE x, i32 LE y, i32 LE z, i32 LE block_state)
//! ```
//!
//! ## Durability strategy
//!
//! Writes to a temporary file (`<dir>/blocks.tmp`) then renames it to the
//! final path (`<dir>/blocks.bin`). On most platforms, rename is atomic,
//! so a crash during write leaves either the old file or the new file,
//! never a corrupted partial write.
//!
//! ## Error handling
//!
//! - Missing file on load -> empty overrides (fresh world)
//! - Corrupt file on load -> empty overrides + log message (no panic)
//! - Write failure -> returned as `Err` (caller decides what to do)

use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// Magic bytes for the block override file format.
const MAGIC: &[u8; 4] = b"RBOV";

/// Current format version.
const FORMAT_VERSION: u32 = 1;

/// Returns the directory path for world data derived from `level_name`.
///
/// The path is `<level_name>/` relative to the current working directory.
/// This is never under `素体データ/`.
pub fn world_dir(level_name: &str) -> PathBuf {
    PathBuf::from(level_name)
}

/// Returns the full path for the block overrides file.
pub fn blocks_file(level_name: &str) -> PathBuf {
    world_dir(level_name).join("blocks.bin")
}

/// Returns the full path for the temporary block overrides file.
fn blocks_tmp_file(level_name: &str) -> PathBuf {
    world_dir(level_name).join("blocks.tmp")
}

/// Serializes block overrides to the v1 binary format.
fn serialize_overrides(overrides: &HashMap<(i32, i32, i32), i32>) -> Vec<u8> {
    let mut buf = Vec::with_capacity(8 + overrides.len() * 16);
    buf.extend_from_slice(MAGIC);
    buf.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    let count = u32::try_from(overrides.len()).unwrap_or(u32::MAX);
    buf.extend_from_slice(&count.to_le_bytes());
    for (&(x, y, z), &block_state) in overrides {
        buf.extend_from_slice(&x.to_le_bytes());
        buf.extend_from_slice(&y.to_le_bytes());
        buf.extend_from_slice(&z.to_le_bytes());
        buf.extend_from_slice(&block_state.to_le_bytes());
    }
    buf
}

/// Deserializes block overrides from the v1 binary format.
///
/// Returns `Ok(overrides)` on success, or `Err` if the data is corrupt.
fn deserialize_overrides(data: &[u8]) -> Result<HashMap<(i32, i32, i32), i32>, PersistError> {
    if data.len() < 12 {
        return Err(PersistError::CorruptFile);
    }
    if &data[0..4] != MAGIC {
        return Err(PersistError::CorruptFile);
    }
    let version = u32::from_le_bytes(
        data[4..8]
            .try_into()
            .map_err(|_| PersistError::CorruptFile)?,
    );
    if version != FORMAT_VERSION {
        return Err(PersistError::UnsupportedVersion(version));
    }
    let count = u32::from_le_bytes(
        data[8..12]
            .try_into()
            .map_err(|_| PersistError::CorruptFile)?,
    ) as usize;
    let expected_len = 12 + count * 16;
    if data.len() < expected_len {
        return Err(PersistError::CorruptFile);
    }
    let mut overrides = HashMap::with_capacity(count);
    let mut offset = 12;
    for _ in 0..count {
        let x = i32::from_le_bytes(
            data[offset..offset + 4]
                .try_into()
                .map_err(|_| PersistError::CorruptFile)?,
        );
        let y = i32::from_le_bytes(
            data[offset + 4..offset + 8]
                .try_into()
                .map_err(|_| PersistError::CorruptFile)?,
        );
        let z = i32::from_le_bytes(
            data[offset + 8..offset + 12]
                .try_into()
                .map_err(|_| PersistError::CorruptFile)?,
        );
        let block_state = i32::from_le_bytes(
            data[offset + 12..offset + 16]
                .try_into()
                .map_err(|_| PersistError::CorruptFile)?,
        );
        overrides.insert((x, y, z), block_state);
        offset += 16;
    }
    Ok(overrides)
}

/// Saves block overrides to disk.
///
/// Writes to a temp file then renames atomically. Creates the directory
/// if it doesn't exist.
pub fn save_overrides(
    level_name: &str,
    overrides: &HashMap<(i32, i32, i32), i32>,
) -> Result<(), PersistError> {
    let dir = world_dir(level_name);
    fs::create_dir_all(&dir).map_err(PersistError::Io)?;
    let tmp_path = blocks_tmp_file(level_name);
    let final_path = blocks_file(level_name);
    let data = serialize_overrides(overrides);
    {
        let mut file = fs::File::create(&tmp_path).map_err(PersistError::Io)?;
        file.write_all(&data).map_err(PersistError::Io)?;
        file.sync_all().map_err(PersistError::Io)?;
    }
    fs::rename(&tmp_path, &final_path).map_err(PersistError::Io)?;
    Ok(())
}

/// Loads block overrides from disk.
///
/// Returns an empty map if the file doesn't exist (fresh world).
/// Returns an empty map and logs if the file is corrupt (safe fallback).
pub fn load_overrides(level_name: &str) -> HashMap<(i32, i32, i32), i32> {
    let path = blocks_file(level_name);
    match load_overrides_from_path(&path) {
        Ok(overrides) => overrides,
        Err(PersistError::FileNotFound) => HashMap::new(),
        Err(e) => {
            eprintln!(
                "warn: failed to load block overrides from {:?}: {}",
                path, e
            );
            HashMap::new()
        }
    }
}

/// Loads block overrides from a specific path (for testing).
fn load_overrides_from_path(path: &Path) -> Result<HashMap<(i32, i32, i32), i32>, PersistError> {
    let mut file = match fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(PersistError::FileNotFound);
        }
        Err(e) => return Err(PersistError::Io(e)),
    };
    let mut data = Vec::new();
    file.read_to_end(&mut data).map_err(PersistError::Io)?;
    deserialize_overrides(&data)
}

/// Errors that can occur during persistence operations.
#[derive(Debug)]
pub enum PersistError {
    /// I/O error.
    Io(std::io::Error),
    /// File not found (only from load).
    FileNotFound,
    /// File exists but is corrupt.
    CorruptFile,
    /// Unsupported format version.
    UnsupportedVersion(u32),
}

impl std::fmt::Display for PersistError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PersistError::Io(e) => write!(f, "I/O error: {}", e),
            PersistError::FileNotFound => write!(f, "file not found"),
            PersistError::CorruptFile => write!(f, "corrupt file"),
            PersistError::UnsupportedVersion(v) => write!(f, "unsupported version {}", v),
        }
    }
}

impl std::error::Error for PersistError {}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Creates a unique temp directory for testing.
    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "rustbound-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = temp_dir();
        let level_name = dir.to_str().unwrap();

        let mut overrides = HashMap::new();
        overrides.insert((0, 64, 0), 1);
        overrides.insert((10, -64, -20), 10);
        overrides.insert((-5, 100, 50), 8);

        save_overrides(level_name, &overrides).unwrap();
        let loaded = load_overrides(level_name);

        assert_eq!(loaded.len(), 3);
        assert_eq!(loaded.get(&(0, 64, 0)), Some(&1));
        assert_eq!(loaded.get(&(10, -64, -20)), Some(&10));
        assert_eq!(loaded.get(&(-5, 100, 50)), Some(&8));

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn load_missing_file_returns_empty() {
        let dir = temp_dir();
        let level_name = dir.to_str().unwrap();

        let loaded = load_overrides(level_name);
        assert!(loaded.is_empty());

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn load_corrupt_file_returns_empty() {
        let dir = temp_dir();
        let level_name = dir.to_str().unwrap();

        // Write garbage to the blocks file
        fs::create_dir_all(world_dir(level_name)).unwrap();
        fs::write(blocks_file(level_name), b"GARBAGE DATA NOT VALID").unwrap();

        let loaded = load_overrides(level_name);
        assert!(
            loaded.is_empty(),
            "corrupt file should yield empty overrides"
        );

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn save_empty_overrides() {
        let dir = temp_dir();
        let level_name = dir.to_str().unwrap();

        let overrides = HashMap::new();
        save_overrides(level_name, &overrides).unwrap();
        let loaded = load_overrides(level_name);
        assert!(loaded.is_empty());

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn overwrite_existing_file() {
        let dir = temp_dir();
        let level_name = dir.to_str().unwrap();

        // Save first version
        let mut overrides1 = HashMap::new();
        overrides1.insert((0, 64, 0), 1);
        save_overrides(level_name, &overrides1).unwrap();

        // Save second version with different data
        let mut overrides2 = HashMap::new();
        overrides2.insert((5, 70, 10), 10);
        overrides2.insert((6, 71, 11), 8);
        save_overrides(level_name, &overrides2).unwrap();

        let loaded = load_overrides(level_name);
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded.get(&(5, 70, 10)), Some(&10));
        assert_eq!(loaded.get(&(0, 64, 0)), None);

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn serialize_deserialize_format() {
        let mut overrides = HashMap::new();
        overrides.insert((1, 2, 3), 4);
        overrides.insert((-1, -2, -3), -4);

        let data = serialize_overrides(&overrides);
        assert_eq!(&data[0..4], MAGIC);
        assert_eq!(u32::from_le_bytes(data[4..8].try_into().unwrap()), 1);
        assert_eq!(u32::from_le_bytes(data[8..12].try_into().unwrap()), 2);

        let deserialized = deserialize_overrides(&data).unwrap();
        assert_eq!(deserialized.len(), 2);
        assert_eq!(deserialized.get(&(1, 2, 3)), Some(&4));
        assert_eq!(deserialized.get(&(-1, -2, -3)), Some(&-4));
    }

    #[test]
    fn deserialize_bad_magic_returns_error() {
        let mut data = Vec::new();
        data.extend_from_slice(b"XXXX");
        data.extend_from_slice(&1u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        let result = deserialize_overrides(&data);
        assert!(matches!(result, Err(PersistError::CorruptFile)));
    }

    #[test]
    fn deserialize_truncated_returns_error() {
        let data = b"RBOV\x01\x00\x00\x00"; // missing count
        let result = deserialize_overrides(data);
        assert!(matches!(result, Err(PersistError::CorruptFile)));
    }

    #[test]
    fn world_dir_never_under_reference_data() {
        let dir = world_dir("myworld");
        assert!(!dir.starts_with("素体データ"));
        assert_eq!(dir, PathBuf::from("myworld"));
    }
}
