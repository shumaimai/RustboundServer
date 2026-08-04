//! Hakoniwa map pack format (H2) and built-in overworld gardens.
//!
//! ## On-disk format (`*.rbpk`, v1)
//!
//! ```text
//! Magic:   4 bytes = "RBPK"
//! Version: u32 LE  = 1
//! Size:    u8      = 0 tiny / 1 small / 2 medium
//! Flags:   u8      = bit0 overworld, bit1 nether, bit2 end (H3+)
//! Surface: f64 LE  = default stand Y
//! Count:   u32 LE  = decoration block entries
//! Entries: Count × (i32 LE x, y, z, block_state)
//! ```
//!
//! Built-in packs are generated in memory (no large assets in the repo).
//! Custom packs can be loaded from `{level}/packs/{size}.rbpk` when present.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::hakoniwa::{DimensionSet, GardenSpec, MapSize};

/// Magic bytes for map pack files.
const MAGIC: &[u8; 4] = b"RBPK";
/// Format version.
const FORMAT_VERSION: u32 = 1;

/// A loaded map pack: garden bounds plus decoration blocks.
#[derive(Debug, Clone)]
pub struct MapPack {
    /// Garden specification (size + border).
    pub garden: GardenSpec,
    /// Decoration that override generated terrain (and are visible via Block Update).
    pub blocks: HashMap<(i32, i32, i32), i32>,
    /// Human-readable pack id (`builtin:tiny`, `file:tiny`, …).
    pub id: String,
}

/// Errors while reading a map pack.
#[derive(Debug)]
pub enum MapPackError {
    /// I/O failure.
    Io(std::io::Error),
    /// File contents are not a valid pack.
    Corrupt,
    /// Size byte does not match an known preset.
    UnknownSize(u8),
}

impl std::fmt::Display for MapPackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "map pack i/o: {e}"),
            Self::Corrupt => write!(f, "map pack is corrupt"),
            Self::UnknownSize(v) => write!(f, "unknown map pack size byte {v}"),
        }
    }
}

impl std::error::Error for MapPackError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for MapPackError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

fn size_to_byte(size: MapSize) -> u8 {
    match size {
        MapSize::Tiny => 0,
        MapSize::Small => 1,
        MapSize::Medium => 2,
    }
}

fn byte_to_size(byte: u8) -> Result<MapSize, MapPackError> {
    match byte {
        0 => Ok(MapSize::Tiny),
        1 => Ok(MapSize::Small),
        2 => Ok(MapSize::Medium),
        other => Err(MapPackError::UnknownSize(other)),
    }
}

/// Serializes a pack to the v1 binary format.
pub fn serialize_pack(pack: &MapPack) -> Vec<u8> {
    let mut buf = Vec::with_capacity(32 + pack.blocks.len() * 16);
    buf.extend_from_slice(MAGIC);
    buf.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    buf.push(size_to_byte(pack.garden.size));
    let mut flags = 0u8;
    if pack.garden.dimensions.overworld {
        flags |= 1;
    }
    if pack.garden.dimensions.nether {
        flags |= 2;
    }
    if pack.garden.dimensions.end {
        flags |= 4;
    }
    buf.push(flags);
    buf.extend_from_slice(&pack.garden.surface_y.to_le_bytes());
    let count = u32::try_from(pack.blocks.len()).unwrap_or(u32::MAX);
    buf.extend_from_slice(&count.to_le_bytes());
    for (&(x, y, z), &state) in &pack.blocks {
        buf.extend_from_slice(&x.to_le_bytes());
        buf.extend_from_slice(&y.to_le_bytes());
        buf.extend_from_slice(&z.to_le_bytes());
        buf.extend_from_slice(&state.to_le_bytes());
    }
    buf
}

/// Deserializes a pack from the v1 binary format.
pub fn deserialize_pack(data: &[u8]) -> Result<MapPack, MapPackError> {
    if data.len() < 4 + 4 + 1 + 1 + 8 + 4 {
        return Err(MapPackError::Corrupt);
    }
    if &data[0..4] != MAGIC {
        return Err(MapPackError::Corrupt);
    }
    let version = u32::from_le_bytes(data[4..8].try_into().map_err(|_| MapPackError::Corrupt)?);
    if version != FORMAT_VERSION {
        return Err(MapPackError::Corrupt);
    }
    let size = byte_to_size(data[8])?;
    let flags = data[9];
    let surface_y = f64::from_le_bytes(data[10..18].try_into().map_err(|_| MapPackError::Corrupt)?);
    let count = u32::from_le_bytes(data[18..22].try_into().map_err(|_| MapPackError::Corrupt)?);
    let mut offset = 22usize;
    let mut blocks = HashMap::new();
    for _ in 0..count {
        if offset + 16 > data.len() {
            return Err(MapPackError::Corrupt);
        }
        let x = i32::from_le_bytes(
            data[offset..offset + 4]
                .try_into()
                .map_err(|_| MapPackError::Corrupt)?,
        );
        let y = i32::from_le_bytes(
            data[offset + 4..offset + 8]
                .try_into()
                .map_err(|_| MapPackError::Corrupt)?,
        );
        let z = i32::from_le_bytes(
            data[offset + 8..offset + 12]
                .try_into()
                .map_err(|_| MapPackError::Corrupt)?,
        );
        let state = i32::from_le_bytes(
            data[offset + 12..offset + 16]
                .try_into()
                .map_err(|_| MapPackError::Corrupt)?,
        );
        blocks.insert((x, y, z), state);
        offset += 16;
    }
    let mut garden = GardenSpec::from_size(size);
    garden.surface_y = surface_y;
    garden.dimensions = DimensionSet {
        overworld: flags & 1 != 0,
        nether: flags & 2 != 0,
        end: flags & 4 != 0,
    };
    Ok(MapPack {
        garden,
        blocks,
        id: format!("file:{}", size.as_str()),
    })
}

/// Loads a pack from a file path.
pub fn load_pack_file(path: &Path) -> Result<MapPack, MapPackError> {
    let data = fs::read(path)?;
    deserialize_pack(&data)
}

/// Writes a pack to a file (creates parent dirs).
pub fn save_pack_file(path: &Path, pack: &MapPack) -> Result<(), MapPackError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serialize_pack(pack))?;
    Ok(())
}

/// Default on-disk pack path: `{level_name}/packs/{size}.rbpk`.
pub fn pack_path(level_name: &str, size: MapSize) -> PathBuf {
    PathBuf::from(level_name)
        .join("packs")
        .join(format!("{}.rbpk", size.as_str()))
}

/// Resolves the active pack: custom file if present, otherwise built-in.
pub fn resolve_pack(level_name: &str, size: MapSize) -> MapPack {
    let path = pack_path(level_name, size);
    match load_pack_file(&path) {
        Ok(mut pack) => {
            // Force configured size bounds even if file metadata differs.
            pack.garden = GardenSpec::from_size(size);
            pack.id = format!("file:{}", size.as_str());
            eprintln!("hakoniwa: loaded map pack from {}", path.display());
            pack
        }
        Err(_) => {
            let pack = builtin_overworld_pack(size);
            eprintln!("hakoniwa: using built-in map pack '{}'", pack.id);
            pack
        }
    }
}

// --- Built-in decoration helpers (1.20.1 global palette states) ---

const STONE: i32 = crate::chunk::STONE_BLOCK_STATE;
const BEDROCK: i32 = crate::chunk::BEDROCK_BLOCK_STATE;
const GRASS: i32 = crate::chunk::GRASS_BLOCK_STATE;
const DIRT: i32 = crate::chunk::DIRT_BLOCK_STATE;
const SAND: i32 = crate::chunk::SAND_BLOCK_STATE;
const DIAMOND_BLOCK: i32 = crate::chunk::DIAMOND_BLOCK_STATE;
const EMERALD_BLOCK: i32 = crate::chunk::EMERALD_BLOCK_STATE;
const COAL_BLOCK: i32 = crate::chunk::COAL_BLOCK_STATE;
const GLOWSTONE: i32 = crate::chunk::GLOWSTONE_BLOCK_STATE;
const NETHERRACK: i32 = crate::chunk::NETHERRACK_BLOCK_STATE;
const END_STONE: i32 = crate::chunk::END_STONE_BLOCK_STATE;
const SOUL_SAND: i32 = crate::chunk::SOUL_SAND_BLOCK_STATE;
const WATER: i32 = crate::fluid::WATER_BLOCK_STATE;
const LAVA: i32 = crate::fluid::LAVA_BLOCK_STATE;
const CHEST: i32 = crate::container::CHEST_BLOCK_STATE;

fn put(blocks: &mut HashMap<(i32, i32, i32), i32>, x: i32, y: i32, z: i32, state: i32) {
    blocks.insert((x, y, z), state);
}

fn fill_disk(
    blocks: &mut HashMap<(i32, i32, i32), i32>,
    cx: i32,
    cy: i32,
    cz: i32,
    radius: i32,
    state: i32,
) {
    for dx in -radius..=radius {
        for dz in -radius..=radius {
            if dx * dx + dz * dz <= radius * radius {
                put(blocks, cx + dx, cy, cz + dz, state);
            }
        }
    }
}

fn border_wall(blocks: &mut HashMap<(i32, i32, i32), i32>, garden: &GardenSpec, height: i32) {
    let y0 = 64;
    for y in y0..y0 + height {
        for x in garden.min_block_x..=garden.max_block_x {
            put(blocks, x, y, garden.min_block_z, BEDROCK);
            put(blocks, x, y, garden.max_block_z, BEDROCK);
        }
        for z in garden.min_block_z..=garden.max_block_z {
            put(blocks, garden.min_block_x, y, z, BEDROCK);
            put(blocks, garden.max_block_x, y, z, BEDROCK);
        }
    }
}

fn spawn_plaza(blocks: &mut HashMap<(i32, i32, i32), i32>, radius: i32) {
    // Replace stone top (y=63) with grass in a disk; stand-level marker at y=64 is air.
    fill_disk(blocks, 0, 63, 0, radius, GRASS);
    // Dirt under a thinner inner disk
    fill_disk(blocks, 0, 62, 0, radius.saturating_sub(1).max(1), DIRT);
}

fn path_east(blocks: &mut HashMap<(i32, i32, i32), i32>, length: i32) {
    for x in 1..=length {
        put(blocks, x, 63, 0, DIRT);
        put(blocks, x, 63, 1, DIRT);
        put(blocks, x, 63, -1, DIRT);
    }
}

/// Digs a shallow static water pond (source blocks, no flow).
fn water_pond(blocks: &mut HashMap<(i32, i32, i32), i32>, cx: i32, cz: i32, radius: i32) {
    for dx in -radius..=radius {
        for dz in -radius..=radius {
            if dx * dx + dz * dz > radius * radius {
                continue;
            }
            // Hollow the plateau and fill with still water.
            put(blocks, cx + dx, 63, cz + dz, WATER);
            put(blocks, cx + dx, 62, cz + dz, WATER);
            put(blocks, cx + dx, 61, cz + dz, DIRT);
        }
    }
}

/// Digs a shallow static lava pool.
fn lava_pool(blocks: &mut HashMap<(i32, i32, i32), i32>, cx: i32, cz: i32, radius: i32) {
    for dx in -radius..=radius {
        for dz in -radius..=radius {
            if dx * dx + dz * dz > radius * radius {
                continue;
            }
            put(blocks, cx + dx, 63, cz + dz, LAVA);
            put(blocks, cx + dx, 62, cz + dz, NETHERRACK);
        }
    }
}

fn portal_pad(
    blocks: &mut HashMap<(i32, i32, i32), i32>,
    cx: i32,
    cz: i32,
    surface_state: i32,
    pad_state: i32,
) {
    for dx in -1..=1 {
        for dz in -1..=1 {
            put(blocks, cx + dx, 63, cz + dz, surface_state);
            put(blocks, cx + dx, 64, cz + dz, pad_state);
        }
    }
}

/// Builds the built-in overworld garden for `size`.
pub fn builtin_overworld_pack(size: MapSize) -> MapPack {
    let garden = GardenSpec::from_size(size);
    let mut blocks = HashMap::new();

    match size {
        MapSize::Tiny => {
            spawn_plaza(&mut blocks, 3);
            border_wall(&mut blocks, &garden, 2);
            put(&mut blocks, 3, 64, 0, DIAMOND_BLOCK);
            put(&mut blocks, 2, 64, 0, STONE);
            put(&mut blocks, -2, 64, 0, STONE);
            put(&mut blocks, 0, 64, 2, STONE);
            put(&mut blocks, 0, 64, -2, STONE);
        }
        MapSize::Small => {
            spawn_plaza(&mut blocks, 5);
            border_wall(&mut blocks, &garden, 2);
            path_east(&mut blocks, 24);
            put(&mut blocks, 3, 64, 0, EMERALD_BLOCK);
            for x in 10..14 {
                for z in 8..12 {
                    put(&mut blocks, x, 63, z, SAND);
                    put(&mut blocks, x, 64, z, SAND);
                }
            }
        }
        MapSize::Medium => {
            spawn_plaza(&mut blocks, 8);
            border_wall(&mut blocks, &garden, 3);
            path_east(&mut blocks, 48);
            put(&mut blocks, 3, 64, 0, COAL_BLOCK);
            for y in 64..69 {
                put(&mut blocks, 20, y, 20, STONE);
            }
            put(&mut blocks, 20, 69, 20, BEDROCK);
            fill_disk(&mut blocks, -30, 63, 30, 4, GRASS);
        }
    }

    // Dimension portals (H3): stand on the pad to transfer.
    // Nether = glowstone west of spawn; End = end stone east of spawn.
    portal_pad(&mut blocks, -6, 0, NETHERRACK, GLOWSTONE);
    portal_pad(&mut blocks, 6, 0, END_STONE, END_STONE);
    // H5: static water pond south of spawn (swim / damp fall).
    water_pond(&mut blocks, 0, -12, 3);
    // H6: starter chest north of spawn.
    put(&mut blocks, 0, 64, 4, CHEST);

    MapPack {
        garden,
        blocks,
        id: format!("builtin:{}:overworld", size.as_str()),
    }
}

/// Built-in nether garden (netherrack plateau + return pad).
pub fn builtin_nether_pack(size: MapSize) -> MapPack {
    let garden = GardenSpec::from_size(size);
    let mut blocks = HashMap::new();
    border_wall(&mut blocks, &garden, 2);
    fill_disk(&mut blocks, 0, 63, 0, 4, SOUL_SAND);
    // Clear spawn column so arrivals at y=65 are not buried.
    for y in 64..=68 {
        put(&mut blocks, 0, y, 0, 0);
        put(&mut blocks, 0, y, 1, 0);
        put(&mut blocks, 1, y, 0, 0);
        put(&mut blocks, -1, y, 0, 0);
        put(&mut blocks, 0, y, -1, 0);
    }
    portal_pad(&mut blocks, -6, 0, NETHERRACK, GLOWSTONE); // return to overworld
    put(&mut blocks, 3, 64, 0, GLOWSTONE);
    // H5: static lava pool (contact damage).
    lava_pool(&mut blocks, 10, 0, 2);
    MapPack {
        garden,
        blocks,
        id: format!("builtin:{}:nether", size.as_str()),
    }
}

/// Built-in end garden (end-stone plateau + return pad + city stub).
pub fn builtin_end_pack(size: MapSize) -> MapPack {
    let garden = GardenSpec::from_size(size);
    let mut blocks = HashMap::new();
    border_wall(&mut blocks, &garden, 2);
    fill_disk(&mut blocks, 0, 63, 0, 5, END_STONE);
    // Clear spawn column for safe arrival at y=65.
    for y in 64..=68 {
        put(&mut blocks, 0, y, 0, 0);
        put(&mut blocks, 0, y, 1, 0);
        put(&mut blocks, 1, y, 0, 0);
        put(&mut blocks, -1, y, 0, 0);
        put(&mut blocks, 0, y, -1, 0);
    }
    // Return pad must be glowstone — end stone is the whole plateau.
    portal_pad(&mut blocks, -6, 0, END_STONE, GLOWSTONE);
    // End-city stub: a few pillars
    for y in 64..72 {
        put(&mut blocks, 12, y, 12, END_STONE);
        put(&mut blocks, 14, y, 12, END_STONE);
        put(&mut blocks, 12, y, 14, END_STONE);
        put(&mut blocks, 14, y, 14, END_STONE);
    }
    put(&mut blocks, 13, 72, 13, DIAMOND_BLOCK);
    MapPack {
        garden,
        blocks,
        id: format!("builtin:{}:end", size.as_str()),
    }
}

/// Built-in pack for any dimension.
pub fn builtin_pack(size: MapSize, dimension: crate::hakoniwa::DimensionId) -> MapPack {
    match dimension {
        crate::hakoniwa::DimensionId::Overworld => builtin_overworld_pack(size),
        crate::hakoniwa::DimensionId::Nether => builtin_nether_pack(size),
        crate::hakoniwa::DimensionId::End => builtin_end_pack(size),
    }
}

/// Default on-disk pack path for a dimension.
pub fn pack_path_for(
    level_name: &str,
    size: MapSize,
    dimension: crate::hakoniwa::DimensionId,
) -> PathBuf {
    let name = match dimension {
        crate::hakoniwa::DimensionId::Overworld => format!("{}.rbpk", size.as_str()),
        other => format!("{}-{}.rbpk", size.as_str(), other.as_str()),
    };
    PathBuf::from(level_name).join("packs").join(name)
}

/// Resolves packs for every dimension (file override, bundled data, or builtin).
pub fn resolve_all_packs(
    level_name: &str,
    size: MapSize,
) -> HashMap<crate::hakoniwa::DimensionId, MapPack> {
    let mut out = HashMap::new();
    for dim in crate::hakoniwa::DimensionId::all() {
        let file_name = match dim {
            crate::hakoniwa::DimensionId::Overworld => format!("{}.rbpk", size.as_str()),
            other => format!("{}-{}.rbpk", size.as_str(), other.as_str()),
        };
        let pack = if let Some(path) = crate::container::resolve_pack_file(level_name, &file_name) {
            match load_pack_file(&path) {
                Ok(mut pack) => {
                    pack.garden = GardenSpec::from_size(size);
                    pack.id = format!("file:{}:{}", size.as_str(), dim.as_str());
                    eprintln!("hakoniwa: loaded {} from {}", pack.id, path.display());
                    pack
                }
                Err(_) => {
                    let pack = builtin_pack(size, dim);
                    eprintln!("hakoniwa: using built-in map pack '{}'", pack.id);
                    pack
                }
            }
        } else {
            let pack = builtin_pack(size, dim);
            eprintln!("hakoniwa: using built-in map pack '{}'", pack.id);
            pack
        };
        out.insert(dim, pack);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_tiny_has_landmark_and_border() {
        let pack = builtin_overworld_pack(MapSize::Tiny);
        assert_eq!(pack.garden.size, MapSize::Tiny);
        assert_eq!(pack.blocks.get(&(3, 64, 0)), Some(&DIAMOND_BLOCK));
        assert_eq!(
            pack.blocks.get(&(pack.garden.min_block_x, 64, 0)),
            Some(&BEDROCK)
        );
        assert!(pack.blocks.len() > 20);
    }

    #[test]
    fn roundtrip_serialization() {
        let pack = builtin_overworld_pack(MapSize::Small);
        let bytes = serialize_pack(&pack);
        let Ok(decoded) = deserialize_pack(&bytes) else {
            panic!("builtin pack must round-trip");
        };
        assert_eq!(decoded.garden.size, MapSize::Small);
        assert_eq!(decoded.blocks.len(), pack.blocks.len());
        assert_eq!(decoded.blocks.get(&(3, 64, 0)), Some(&EMERALD_BLOCK));
    }

    #[test]
    fn three_sizes_are_distinct() {
        let t = builtin_overworld_pack(MapSize::Tiny);
        let s = builtin_overworld_pack(MapSize::Small);
        let m = builtin_overworld_pack(MapSize::Medium);
        assert!(m.blocks.len() > s.blocks.len());
        assert!(s.blocks.len() > t.blocks.len());
        assert_ne!(t.blocks.get(&(3, 64, 0)), s.blocks.get(&(3, 64, 0)));
    }

    #[test]
    fn overworld_has_water_pond_and_nether_has_lava() {
        let ow = builtin_overworld_pack(MapSize::Tiny);
        assert_eq!(ow.blocks.get(&(0, 63, -12)), Some(&WATER));
        assert_eq!(ow.blocks.get(&(0, 64, 4)), Some(&CHEST));
        let n = builtin_nether_pack(MapSize::Tiny);
        assert_eq!(n.blocks.get(&(10, 63, 0)), Some(&LAVA));
    }

    #[test]
    fn resolve_pack_falls_back_to_builtin() {
        let pack = resolve_pack("/tmp/rustbound-no-such-level-h2", MapSize::Tiny);
        assert!(pack.id.starts_with("builtin:"));
    }

    #[test]
    fn nether_and_end_packs_have_return_pads() {
        let n = builtin_nether_pack(MapSize::Tiny);
        assert_eq!(n.blocks.get(&(-6, 64, 0)), Some(&GLOWSTONE));
        let e = builtin_end_pack(MapSize::Tiny);
        assert_eq!(e.blocks.get(&(-6, 64, 0)), Some(&GLOWSTONE));
        assert_eq!(e.blocks.get(&(0, 65, 0)), Some(&0));
    }

    #[test]
    fn overworld_has_portal_pads() {
        let pack = builtin_overworld_pack(MapSize::Tiny);
        assert_eq!(pack.blocks.get(&(-6, 64, 0)), Some(&GLOWSTONE));
        assert_eq!(pack.blocks.get(&(6, 64, 0)), Some(&END_STONE));
    }

    #[test]
    fn resolve_all_packs_covers_three_dims() {
        let packs = resolve_all_packs("/tmp/rustbound-no-such-level-h3", MapSize::Tiny);
        assert_eq!(packs.len(), 3);
        assert!(
            packs[&crate::hakoniwa::DimensionId::Nether]
                .id
                .contains("nether")
        );
    }
}
