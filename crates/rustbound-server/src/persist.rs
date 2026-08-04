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

/// Block override path for a hakoniwa dimension.
pub fn blocks_file_for(level_name: &str, dimension: crate::hakoniwa::DimensionId) -> PathBuf {
    match dimension {
        crate::hakoniwa::DimensionId::Overworld => blocks_file(level_name),
        crate::hakoniwa::DimensionId::Nether => world_dir(level_name).join("blocks-nether.bin"),
        crate::hakoniwa::DimensionId::End => world_dir(level_name).join("blocks-end.bin"),
    }
}

/// Returns the full path for the temporary block overrides file.
fn blocks_tmp_file(level_name: &str) -> PathBuf {
    world_dir(level_name).join("blocks.tmp")
}

fn blocks_tmp_file_for(level_name: &str, dimension: crate::hakoniwa::DimensionId) -> PathBuf {
    match dimension {
        crate::hakoniwa::DimensionId::Overworld => blocks_tmp_file(level_name),
        crate::hakoniwa::DimensionId::Nether => world_dir(level_name).join("blocks-nether.tmp"),
        crate::hakoniwa::DimensionId::End => world_dir(level_name).join("blocks-end.tmp"),
    }
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
    save_overrides_for(
        level_name,
        crate::hakoniwa::DimensionId::Overworld,
        overrides,
    )
}

/// Saves dig/place overrides for a specific dimension.
pub fn save_overrides_for(
    level_name: &str,
    dimension: crate::hakoniwa::DimensionId,
    overrides: &HashMap<(i32, i32, i32), i32>,
) -> Result<(), PersistError> {
    let dir = world_dir(level_name);
    fs::create_dir_all(&dir).map_err(PersistError::Io)?;
    let tmp_path = blocks_tmp_file_for(level_name, dimension);
    let final_path = blocks_file_for(level_name, dimension);
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
    load_overrides_for(level_name, crate::hakoniwa::DimensionId::Overworld)
}

/// Loads dig/place overrides for a specific dimension.
pub fn load_overrides_for(
    level_name: &str,
    dimension: crate::hakoniwa::DimensionId,
) -> HashMap<(i32, i32, i32), i32> {
    let path = blocks_file_for(level_name, dimension);
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

// ---------------------------------------------------------------------------
// Player data persistence
// ---------------------------------------------------------------------------

/// Per-player data that is saved and restored across reconnects.
///
/// Keyed by offline UUID. Contains position, rotation, gamemode, inventory,
/// held slot, and vitals (health/food/saturation).
#[derive(Debug, Clone)]
pub struct PlayerData {
    /// The player's X coordinate.
    pub x: f64,
    /// The player's Y coordinate.
    pub y: f64,
    /// The player's Z coordinate.
    pub z: f64,
    /// The player's yaw (degrees).
    pub yaw: f32,
    /// The player's pitch (degrees).
    pub pitch: f32,
    /// The player's gamemode (0=Survival, 1=Creative, 2=Adventure, 3=Spectator).
    pub gamemode: u8,
    /// The player's held hotbar slot (0-8).
    pub held_slot: u8,
    /// The player's health (0 or less = dead, 20 = full HP).
    pub health: f32,
    /// The player's food level (0-20).
    pub food: i32,
    /// The player's food saturation (0.0 to 5.0).
    pub food_saturation: f32,
    /// The player's inventory slots (present, item_id, count, nbt_len, nbt_bytes).
    pub slots: Vec<(bool, i32, i8, Vec<u8>)>,
}

/// Magic bytes for the player data file format.
const PLAYER_MAGIC: &[u8; 4] = b"RBPD";

/// Current player data format version.
const PLAYER_FORMAT_VERSION: u32 = 1;

/// Returns the full path for the player data file.
pub fn players_file(level_name: &str) -> PathBuf {
    world_dir(level_name).join("players.bin")
}

/// Returns the full path for the temporary player data file.
fn players_tmp_file(level_name: &str) -> PathBuf {
    world_dir(level_name).join("players.tmp")
}

/// Serializes player data map to the v1 binary format.
///
/// Format:
/// ```text
/// Magic:   "RBPD"
/// Version: u32 LE = 1
/// Count:   u32 LE = number of players
/// Per player:
///   UUID:   16 bytes (big-endian, matching Uuid::to_be_bytes)
///   x, y, z: f64 LE * 3
///   yaw, pitch: f32 LE * 2
///   gamemode: u8
///   held_slot: u8
///   health, food_saturation: f32 LE * 2
///   food: i32 LE
///   slot_count: u32 LE
///   Per slot:
///     present: u8 (0 or 1)
///     item_id: i32 LE
///     count: i8
///     nbt_len: u32 LE
///     nbt_bytes: nbt_len bytes
/// ```
fn serialize_players(
    players: &HashMap<rustbound_protocol::primitives::Uuid, PlayerData>,
) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(PLAYER_MAGIC);
    buf.extend_from_slice(&PLAYER_FORMAT_VERSION.to_le_bytes());
    let count = u32::try_from(players.len()).unwrap_or(u32::MAX);
    buf.extend_from_slice(&count.to_le_bytes());
    for (uuid, data) in players {
        buf.extend_from_slice(&uuid.to_be_bytes());
        buf.extend_from_slice(&data.x.to_le_bytes());
        buf.extend_from_slice(&data.y.to_le_bytes());
        buf.extend_from_slice(&data.z.to_le_bytes());
        buf.extend_from_slice(&data.yaw.to_le_bytes());
        buf.extend_from_slice(&data.pitch.to_le_bytes());
        buf.push(data.gamemode);
        buf.push(data.held_slot);
        buf.extend_from_slice(&data.health.to_le_bytes());
        buf.extend_from_slice(&data.food_saturation.to_le_bytes());
        buf.extend_from_slice(&data.food.to_le_bytes());
        let slot_count = u32::try_from(data.slots.len()).unwrap_or(u32::MAX);
        buf.extend_from_slice(&slot_count.to_le_bytes());
        for (present, item_id, count, nbt) in &data.slots {
            buf.push(if *present { 1 } else { 0 });
            buf.extend_from_slice(&item_id.to_le_bytes());
            buf.extend_from_slice(&count.to_le_bytes());
            let nbt_len = u32::try_from(nbt.len()).unwrap_or(u32::MAX);
            buf.extend_from_slice(&nbt_len.to_le_bytes());
            buf.extend_from_slice(nbt);
        }
    }
    buf
}

/// Deserializes player data from the v1 binary format.
fn deserialize_players(
    data: &[u8],
) -> Result<HashMap<rustbound_protocol::primitives::Uuid, PlayerData>, PersistError> {
    use rustbound_protocol::primitives::Uuid;
    if data.len() < 12 {
        return Err(PersistError::CorruptFile);
    }
    if &data[0..4] != PLAYER_MAGIC {
        return Err(PersistError::CorruptFile);
    }
    let version = u32::from_le_bytes(
        data[4..8]
            .try_into()
            .map_err(|_| PersistError::CorruptFile)?,
    );
    if version != PLAYER_FORMAT_VERSION {
        return Err(PersistError::UnsupportedVersion(version));
    }
    let count = u32::from_le_bytes(
        data[8..12]
            .try_into()
            .map_err(|_| PersistError::CorruptFile)?,
    ) as usize;
    let mut players = HashMap::with_capacity(count);
    let mut offset = 12;
    for _ in 0..count {
        // UUID(16) + x(8) + y(8) + z(8) + yaw(4) + pitch(4) + gamemode(1) + held_slot(1) + health(4) + food_saturation(4) + food(4) + slot_count(4) = 66
        if offset + 66 > data.len() {
            return Err(PersistError::CorruptFile);
        }
        let uuid_bytes: [u8; 16] = data[offset..offset + 16]
            .try_into()
            .map_err(|_| PersistError::CorruptFile)?;
        let uuid = Uuid::from_be_bytes(uuid_bytes);
        offset += 16;
        let x = f64::from_le_bytes(
            data[offset..offset + 8]
                .try_into()
                .map_err(|_| PersistError::CorruptFile)?,
        );
        offset += 8;
        let y = f64::from_le_bytes(
            data[offset..offset + 8]
                .try_into()
                .map_err(|_| PersistError::CorruptFile)?,
        );
        offset += 8;
        let z = f64::from_le_bytes(
            data[offset..offset + 8]
                .try_into()
                .map_err(|_| PersistError::CorruptFile)?,
        );
        offset += 8;
        let yaw = f32::from_le_bytes(
            data[offset..offset + 4]
                .try_into()
                .map_err(|_| PersistError::CorruptFile)?,
        );
        offset += 4;
        let pitch = f32::from_le_bytes(
            data[offset..offset + 4]
                .try_into()
                .map_err(|_| PersistError::CorruptFile)?,
        );
        offset += 4;
        let gamemode = data[offset];
        offset += 1;
        let held_slot = data[offset];
        offset += 1;
        let health = f32::from_le_bytes(
            data[offset..offset + 4]
                .try_into()
                .map_err(|_| PersistError::CorruptFile)?,
        );
        offset += 4;
        let food_saturation = f32::from_le_bytes(
            data[offset..offset + 4]
                .try_into()
                .map_err(|_| PersistError::CorruptFile)?,
        );
        offset += 4;
        let food = i32::from_le_bytes(
            data[offset..offset + 4]
                .try_into()
                .map_err(|_| PersistError::CorruptFile)?,
        );
        offset += 4;
        if offset + 4 > data.len() {
            return Err(PersistError::CorruptFile);
        }
        let slot_count = u32::from_le_bytes(
            data[offset..offset + 4]
                .try_into()
                .map_err(|_| PersistError::CorruptFile)?,
        ) as usize;
        offset += 4;
        let mut slots = Vec::with_capacity(slot_count);
        for _ in 0..slot_count {
            if offset + 1 + 4 + 1 + 4 > data.len() {
                return Err(PersistError::CorruptFile);
            }
            let present = data[offset] != 0;
            offset += 1;
            let item_id = i32::from_le_bytes(
                data[offset..offset + 4]
                    .try_into()
                    .map_err(|_| PersistError::CorruptFile)?,
            );
            offset += 4;
            let slot_count_byte = data[offset];
            offset += 1;
            let nbt_len = u32::from_le_bytes(
                data[offset..offset + 4]
                    .try_into()
                    .map_err(|_| PersistError::CorruptFile)?,
            ) as usize;
            offset += 4;
            if offset + nbt_len > data.len() {
                return Err(PersistError::CorruptFile);
            }
            let nbt = data[offset..offset + nbt_len].to_vec();
            offset += nbt_len;
            slots.push((present, item_id, slot_count_byte as i8, nbt));
        }
        players.insert(
            uuid,
            PlayerData {
                x,
                y,
                z,
                yaw,
                pitch,
                gamemode,
                held_slot,
                health,
                food,
                food_saturation,
                slots,
            },
        );
    }
    Ok(players)
}

/// Saves player data to disk.
///
/// Writes to a temp file then renames atomically.
pub fn save_players(
    level_name: &str,
    players: &HashMap<rustbound_protocol::primitives::Uuid, PlayerData>,
) -> Result<(), PersistError> {
    let dir = world_dir(level_name);
    fs::create_dir_all(&dir).map_err(PersistError::Io)?;
    let tmp_path = players_tmp_file(level_name);
    let final_path = players_file(level_name);
    let data = serialize_players(players);
    {
        let mut file = fs::File::create(&tmp_path).map_err(PersistError::Io)?;
        file.write_all(&data).map_err(PersistError::Io)?;
        file.sync_all().map_err(PersistError::Io)?;
    }
    fs::rename(&tmp_path, &final_path).map_err(PersistError::Io)?;
    Ok(())
}

/// Loads player data from disk.
///
/// Returns an empty map if the file doesn't exist (fresh world).
/// Returns an empty map and logs if the file is corrupt (safe fallback).
pub fn load_players(level_name: &str) -> HashMap<rustbound_protocol::primitives::Uuid, PlayerData> {
    let path = players_file(level_name);
    match load_players_from_path(&path) {
        Ok(players) => players,
        Err(PersistError::FileNotFound) => HashMap::new(),
        Err(e) => {
            eprintln!("warn: failed to load player data from {:?}: {}", path, e);
            HashMap::new()
        }
    }
}

/// Loads player data from a specific path (for testing).
fn load_players_from_path(
    path: &Path,
) -> Result<HashMap<rustbound_protocol::primitives::Uuid, PlayerData>, PersistError> {
    let mut file = match fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(PersistError::FileNotFound);
        }
        Err(e) => return Err(PersistError::Io(e)),
    };
    let mut data = Vec::new();
    file.read_to_end(&mut data).map_err(PersistError::Io)?;
    deserialize_players(&data)
}

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

    // --- Player data tests ---

    use rustbound_protocol::primitives::Uuid;

    fn sample_player_data() -> PlayerData {
        PlayerData {
            x: 128.5,
            y: 64.0,
            z: -32.25,
            yaw: 90.0,
            pitch: -45.0,
            gamemode: 1,
            held_slot: 3,
            health: 15.5,
            food: 18,
            food_saturation: 3.5,
            slots: vec![
                (true, 1, 64, Vec::new()),       // stone x64
                (false, 0, 0, Vec::new()),       // empty
                (true, 10, 1, vec![0x08, 0x00]), // dirt with some NBT
            ],
        }
    }

    #[test]
    fn save_and_load_player_roundtrip() {
        let dir = temp_dir();
        let level_name = dir.to_str().unwrap();

        let uuid1 = Uuid::from_be_bytes([1; 16]);
        let uuid2 = Uuid::from_be_bytes([2; 16]);
        let mut players = HashMap::new();
        players.insert(uuid1, sample_player_data());
        players.insert(
            uuid2,
            PlayerData {
                x: 0.0,
                y: 100.0,
                z: 0.0,
                yaw: 0.0,
                pitch: 0.0,
                gamemode: 0,
                held_slot: 0,
                health: 20.0,
                food: 20,
                food_saturation: 5.0,
                slots: vec![],
            },
        );

        save_players(level_name, &players).unwrap();
        let loaded = load_players(level_name);

        assert_eq!(loaded.len(), 2);
        let p1 = loaded.get(&uuid1).unwrap();
        assert_eq!(p1.x, 128.5);
        assert_eq!(p1.y, 64.0);
        assert_eq!(p1.z, -32.25);
        assert_eq!(p1.yaw, 90.0);
        assert_eq!(p1.pitch, -45.0);
        assert_eq!(p1.gamemode, 1);
        assert_eq!(p1.held_slot, 3);
        assert_eq!(p1.health, 15.5);
        assert_eq!(p1.food, 18);
        assert_eq!(p1.food_saturation, 3.5);
        assert_eq!(p1.slots.len(), 3);
        assert_eq!(p1.slots[0], (true, 1, 64, Vec::new()));
        assert_eq!(p1.slots[1], (false, 0, 0, Vec::new()));
        assert_eq!(p1.slots[2], (true, 10, 1, vec![0x08, 0x00]));

        let p2 = loaded.get(&uuid2).unwrap();
        assert_eq!(p2.gamemode, 0);
        assert!(p2.slots.is_empty());

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn load_missing_players_returns_empty() {
        let dir = temp_dir();
        let level_name = dir.to_str().unwrap();
        let loaded = load_players(level_name);
        assert!(loaded.is_empty());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn load_corrupt_players_returns_empty() {
        let dir = temp_dir();
        let level_name = dir.to_str().unwrap();
        fs::create_dir_all(world_dir(level_name)).unwrap();
        fs::write(players_file(level_name), b"CORRUPT GARBAGE").unwrap();
        let loaded = load_players(level_name);
        assert!(loaded.is_empty());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn save_empty_players() {
        let dir = temp_dir();
        let level_name = dir.to_str().unwrap();
        let players = HashMap::new();
        save_players(level_name, &players).unwrap();
        let loaded = load_players(level_name);
        assert!(loaded.is_empty());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn player_data_uses_offline_uuid_key() {
        use crate::offline_uuid::offline_uuid_from_username;
        let dir = temp_dir();
        let level_name = dir.to_str().unwrap();

        let uuid = offline_uuid_from_username("Steve");
        let mut players = HashMap::new();
        players.insert(uuid, sample_player_data());

        save_players(level_name, &players).unwrap();
        let loaded = load_players(level_name);

        // Same username -> same UUID -> data restored
        let restored = loaded.get(&offline_uuid_from_username("Steve"));
        assert!(restored.is_some());
        assert_eq!(restored.unwrap().gamemode, 1);

        fs::remove_dir_all(&dir).ok();
    }
}
