//! Hakoniwa H6: minimal chest containers.
//!
//! Single chests (`generic_9x3`) can be opened, clicked, and closed. Contents
//! are stored per block position and optionally persisted beside world blocks.

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use rustbound_protocol::play::Slot;

use crate::hakoniwa::DimensionId;
use crate::persist::world_dir;

/// Still chest block state (1.20.1 default: facing north, type single).
pub const CHEST_BLOCK_STATE: i32 = 2955;
/// Inclusive max chest block state ID in 1.20.1.
pub const CHEST_STATE_MAX: i32 = 2977;
/// Inclusive min chest block state ID in 1.20.1.
pub const CHEST_STATE_MIN: i32 = 2954;
/// Chest item ID (1.20.1 `minecraft:chest`).
pub const CHEST_ITEM_ID: i32 = 277;
/// Menu type registry ID for `generic_9x3` (single chest / barrel).
pub const MENU_TYPE_GENERIC_9X3: i32 = 2;
/// Slots in a single chest inventory.
pub const CHEST_SLOT_COUNT: usize = 27;
/// Total slots in an open chest window (chest + player main + hotbar).
pub const CHEST_WINDOW_SLOT_COUNT: usize = 63;
/// JSON chat title for the chest GUI.
pub const CHEST_TITLE: &str = "{\"text\":\"Chest\"}";

const MAGIC: &[u8; 4] = b"RCHS";
const FORMAT_VERSION: u32 = 1;

/// World position key for a container block.
pub type ContainerPos = (DimensionId, i32, i32, i32);

/// True when `block_state` is any chest variant in the 1.20.1 palette range.
pub fn is_chest(block_state: i32) -> bool {
    (CHEST_STATE_MIN..=CHEST_STATE_MAX).contains(&block_state)
}

/// Maps a `generic_9x3` window slot to a player inventory slot (0–40), if any.
///
/// Window layout: 0–26 chest, 27–53 player main (inv 9–35), 54–62 hotbar (inv 0–8).
pub fn window_slot_to_player_slot(window_slot: i16) -> Option<usize> {
    match window_slot {
        27..=53 => Some((window_slot - 27 + 9) as usize),
        54..=62 => Some((window_slot - 54) as usize),
        _ => None,
    }
}

/// True when the window slot indexes the chest section.
pub fn is_chest_window_slot(window_slot: i16) -> bool {
    (0..CHEST_SLOT_COUNT as i16).contains(&window_slot)
}

/// Builds the 63-slot window content for an open chest.
pub fn build_chest_window_slots(chest: &[Slot; CHEST_SLOT_COUNT], player: &[Slot]) -> Vec<Slot> {
    let mut slots = Vec::with_capacity(CHEST_WINDOW_SLOT_COUNT);
    slots.extend_from_slice(chest);
    // Player main inventory (9–35)
    for i in 9..36 {
        slots.push(player.get(i).cloned().unwrap_or_else(Slot::empty));
    }
    // Hotbar (0–8)
    for i in 0..9 {
        slots.push(player.get(i).cloned().unwrap_or_else(Slot::empty));
    }
    slots
}

/// Applies a window-slot write into chest storage and/or player inventory.
pub fn apply_window_slot(
    chest: &mut [Slot; CHEST_SLOT_COUNT],
    player: &mut [Slot],
    window_slot: i16,
    item: Slot,
) {
    if is_chest_window_slot(window_slot) {
        chest[window_slot as usize] = item;
        return;
    }
    if let Some(player_slot) = window_slot_to_player_slot(window_slot) {
        if player_slot < player.len() {
            player[player_slot] = item;
        }
    }
}

/// Starter loot placed in built-in garden chests.
pub fn starter_chest_loot() -> [Slot; CHEST_SLOT_COUNT] {
    let mut slots = std::array::from_fn(|_| Slot::empty());
    // A few useful stubs from the server registry (dirt / stone / oak planks).
    slots[0] = Slot::item(10, 16); // dirt
    slots[1] = Slot::item(1, 16); // stone
    slots[2] = Slot::item(15, 8); // oak planks (registry stub)
    slots[13] = Slot::item(CHEST_ITEM_ID, 1);
    slots
}

/// In-memory chest contents keyed by dimension + block position.
#[derive(Debug, Default, Clone)]
pub struct ContainerStore {
    chests: HashMap<ContainerPos, [Slot; CHEST_SLOT_COUNT]>,
}

impl ContainerStore {
    /// Empty store.
    pub fn new() -> Self {
        Self {
            chests: HashMap::new(),
        }
    }

    /// Returns chest slots, creating starter loot on first access when `seed` is true.
    pub fn chest_mut(&mut self, pos: ContainerPos, seed: bool) -> &mut [Slot; CHEST_SLOT_COUNT] {
        self.chests.entry(pos).or_insert_with(|| {
            if seed {
                starter_chest_loot()
            } else {
                std::array::from_fn(|_| Slot::empty())
            }
        })
    }

    /// Read-only view of chest slots if present.
    pub fn chest(&self, pos: ContainerPos) -> Option<&[Slot; CHEST_SLOT_COUNT]> {
        self.chests.get(&pos)
    }

    /// Removes chest contents when the block is destroyed.
    pub fn remove(&mut self, pos: ContainerPos) {
        self.chests.remove(&pos);
    }

    /// Ensures a seeded chest exists at `pos` (used when placing map-pack chests).
    pub fn ensure_seeded(&mut self, pos: ContainerPos) {
        self.chests
            .entry(pos)
            .or_insert_with(starter_chest_loot);
    }

    /// Number of stored chests (tests / diagnostics).
    pub fn len(&self) -> usize {
        self.chests.len()
    }

    /// True when no chests are stored.
    pub fn is_empty(&self) -> bool {
        self.chests.is_empty()
    }
}

/// Path for persisted chest contents.
pub fn chests_file(level_name: &str) -> PathBuf {
    world_dir(level_name).join("chests.bin")
}

fn encode_slot_bytes(slot: &Slot, buf: &mut Vec<u8>) {
    if slot.present {
        buf.push(1);
        buf.extend_from_slice(&slot.item_id.to_le_bytes());
        buf.push(slot.count as u8);
        let nbt_len = u16::try_from(slot.nbt.len()).unwrap_or(u16::MAX);
        buf.extend_from_slice(&nbt_len.to_le_bytes());
        let n = nbt_len as usize;
        buf.extend_from_slice(&slot.nbt[..n.min(slot.nbt.len())]);
    } else {
        buf.push(0);
    }
}

fn decode_slot_bytes(input: &mut &[u8]) -> Option<Slot> {
    if input.is_empty() {
        return None;
    }
    let present = input[0];
    *input = &input[1..];
    if present == 0 {
        return Some(Slot::empty());
    }
    if input.len() < 4 + 1 + 2 {
        return None;
    }
    let item_id = i32::from_le_bytes(input[0..4].try_into().ok()?);
    let count = input[4] as i8;
    let nbt_len = u16::from_le_bytes(input[5..7].try_into().ok()?) as usize;
    *input = &input[7..];
    if input.len() < nbt_len {
        return None;
    }
    let nbt = input[..nbt_len].to_vec();
    *input = &input[nbt_len..];
    Some(Slot::with_nbt(item_id, count, nbt))
}

fn dimension_to_byte(dim: DimensionId) -> u8 {
    match dim {
        DimensionId::Overworld => 0,
        DimensionId::Nether => 1,
        DimensionId::End => 2,
    }
}

fn byte_to_dimension(byte: u8) -> Option<DimensionId> {
    match byte {
        0 => Some(DimensionId::Overworld),
        1 => Some(DimensionId::Nether),
        2 => Some(DimensionId::End),
        _ => None,
    }
}

/// Serializes chest contents to a compact binary blob.
pub fn serialize_chests(store: &ContainerStore) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(MAGIC);
    buf.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    let count = u32::try_from(store.chests.len()).unwrap_or(u32::MAX);
    buf.extend_from_slice(&count.to_le_bytes());
    for (&(dim, x, y, z), slots) in &store.chests {
        buf.push(dimension_to_byte(dim));
        buf.extend_from_slice(&x.to_le_bytes());
        buf.extend_from_slice(&y.to_le_bytes());
        buf.extend_from_slice(&z.to_le_bytes());
        for slot in slots {
            encode_slot_bytes(slot, &mut buf);
        }
    }
    buf
}

/// Deserializes chest contents; returns empty store on corruption.
pub fn deserialize_chests(bytes: &[u8]) -> ContainerStore {
    let mut store = ContainerStore::new();
    if bytes.len() < 12 || &bytes[0..4] != MAGIC {
        return store;
    }
    let version = u32::from_le_bytes(bytes[4..8].try_into().unwrap_or([0; 4]));
    if version != FORMAT_VERSION {
        return store;
    }
    let count = u32::from_le_bytes(bytes[8..12].try_into().unwrap_or([0; 4])) as usize;
    let mut input = &bytes[12..];
    for _ in 0..count {
        if input.is_empty() {
            break;
        }
        let dim = match byte_to_dimension(input[0]) {
            Some(d) => d,
            None => break,
        };
        input = &input[1..];
        if input.len() < 12 {
            break;
        }
        let x = i32::from_le_bytes(input[0..4].try_into().unwrap_or([0; 4]));
        let y = i32::from_le_bytes(input[4..8].try_into().unwrap_or([0; 4]));
        let z = i32::from_le_bytes(input[8..12].try_into().unwrap_or([0; 4]));
        input = &input[12..];
        let mut slots = std::array::from_fn(|_| Slot::empty());
        let mut ok = true;
        for slot in &mut slots {
            match decode_slot_bytes(&mut input) {
                Some(s) => *slot = s,
                None => {
                    ok = false;
                    break;
                }
            }
        }
        if !ok {
            break;
        }
        store.chests.insert((dim, x, y, z), slots);
    }
    store
}

/// Loads chests from `{level}/chests.bin`, or empty if missing/corrupt.
pub fn load_chests(level_name: &str) -> ContainerStore {
    let path = chests_file(level_name);
    match fs::read(&path) {
        Ok(bytes) => {
            let store = deserialize_chests(&bytes);
            eprintln!(
                "hakoniwa: loaded {} chest(s) from {}",
                store.len(),
                path.display()
            );
            store
        }
        Err(_) => ContainerStore::new(),
    }
}

/// Atomically writes chests to `{level}/chests.bin`.
pub fn save_chests(level_name: &str, store: &ContainerStore) -> std::io::Result<()> {
    let dir = world_dir(level_name);
    fs::create_dir_all(&dir)?;
    let final_path = chests_file(level_name);
    let tmp_path = dir.join("chests.tmp");
    let bytes = serialize_chests(store);
    {
        let mut f = fs::File::create(&tmp_path)?;
        f.write_all(&bytes)?;
        f.sync_all()?;
    }
    fs::rename(&tmp_path, &final_path)?;
    Ok(())
}

/// Bundled pack directory shipped with the server (`data/hakoniwa/packs`).
pub fn bundled_packs_dir() -> PathBuf {
    PathBuf::from("data/hakoniwa/packs")
}

/// Resolves a pack path preferring level overrides, then bundled data.
pub fn resolve_pack_file(level_name: &str, file_name: &str) -> Option<PathBuf> {
    let level_path = PathBuf::from(level_name).join("packs").join(file_name);
    if level_path.is_file() {
        return Some(level_path);
    }
    let bundled = bundled_packs_dir().join(file_name);
    if bundled.is_file() {
        return Some(bundled);
    }
    None
}

/// Writes all built-in size/dimension packs under `dir` (for shipping / size checks).
pub fn write_bundled_packs(dir: &Path) -> std::io::Result<usize> {
    use crate::hakoniwa::{DimensionId, MapSize};
    use crate::map_pack::{builtin_pack, serialize_pack};

    fs::create_dir_all(dir)?;
    let mut written = 0usize;
    for size in [MapSize::Tiny, MapSize::Small, MapSize::Medium] {
        for dim in [
            DimensionId::Overworld,
            DimensionId::Nether,
            DimensionId::End,
        ] {
            let pack = builtin_pack(size, dim);
            let name = match dim {
                DimensionId::Overworld => format!("{}.rbpk", size.as_str()),
                other => format!("{}-{}.rbpk", size.as_str(), other.as_str()),
            };
            let path = dir.join(name);
            fs::write(&path, serialize_pack(&pack))?;
            written += 1;
        }
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustbound_protocol::play::Slot;

    #[test]
    fn is_chest_recognizes_default_state() {
        assert!(is_chest(CHEST_BLOCK_STATE));
        assert!(is_chest(CHEST_STATE_MIN));
        assert!(is_chest(CHEST_STATE_MAX));
        assert!(!is_chest(1));
        assert!(!is_chest(0));
    }

    #[test]
    fn window_slot_mapping() {
        assert_eq!(window_slot_to_player_slot(0), None);
        assert_eq!(window_slot_to_player_slot(27), Some(9));
        assert_eq!(window_slot_to_player_slot(53), Some(35));
        assert_eq!(window_slot_to_player_slot(54), Some(0));
        assert_eq!(window_slot_to_player_slot(62), Some(8));
    }

    #[test]
    fn build_window_has_63_slots() {
        let chest = starter_chest_loot();
        let player: Vec<Slot> = (0..46).map(|_| Slot::empty()).collect();
        let window = build_chest_window_slots(&chest, &player);
        assert_eq!(window.len(), CHEST_WINDOW_SLOT_COUNT);
        assert!(window[0].present);
        assert_eq!(window[0].item_id, 10);
    }

    #[test]
    fn apply_window_slot_writes_chest_and_player() {
        let mut chest = std::array::from_fn(|_| Slot::empty());
        let mut player: Vec<Slot> = (0..46).map(|_| Slot::empty()).collect();
        apply_window_slot(&mut chest, &mut player, 0, Slot::item(1, 2));
        apply_window_slot(&mut chest, &mut player, 54, Slot::item(10, 3));
        assert_eq!(chest[0].item_id, 1);
        assert_eq!(player[0].item_id, 10);
        assert_eq!(player[0].count, 3);
    }

    #[test]
    fn serialize_roundtrip() {
        let mut store = ContainerStore::new();
        store.ensure_seeded((DimensionId::Overworld, 0, 64, 4));
        let bytes = serialize_chests(&store);
        let loaded = deserialize_chests(&bytes);
        assert_eq!(loaded.len(), 1);
        let Some(slots) = loaded.chest((DimensionId::Overworld, 0, 64, 4)) else {
            panic!("chest present after roundtrip");
        };
        assert!(slots[0].present);
    }

    #[test]
    fn write_bundled_packs_creates_nine_files() {
        let dir = std::env::temp_dir().join(format!(
            "rustbound-h6-packs-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        let n = match write_bundled_packs(&dir) {
            Ok(n) => n,
            Err(e) => panic!("write packs to temp dir: {e}"),
        };
        assert_eq!(n, 9);
        assert!(dir.join("tiny.rbpk").is_file());
        assert!(dir.join("medium-end.rbpk").is_file());
        let _ = fs::remove_dir_all(&dir);
    }
}
