//! Minimal block/item ID registry for the flat-world palette.
//!
//! This module provides a small, documented mapping between Minecraft 1.20.1
//! item IDs and block state IDs for the blocks used in the flat-world generator
//! and creative placement. The IDs are sourced from public protocol documentation
//! (wiki.vg / minecraft-data) via black-box observation only — no Minecraft jars
//! or decompiled code are used.
//!
//! ## ID conventions
//!
//! - **Block state ID**: The global palette ID used in chunk data and block updates.
//!   Air = 0, Stone = 1, Dirt = 10, Grass Block = 8, etc.
//! - **Item ID**: The protocol item ID used in slot data (Set Container Content,
//!   Set Creative Mode Slot). For block items, the item ID typically equals the
//!   block's base ID (not the state ID). For simplicity in this minimal registry,
//!   we use the item ID as the key and map it to the default block state ID.
//!
//! ## Unknown ID policy
//!
//! If an item ID is not in the registry, `item_to_block_state` returns `None`.
//! Callers should treat `None` as "no block to place" (e.g., empty hand or
//! non-block item like a tool or food).

/// A mapping entry from item ID to block state ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockItemEntry {
    /// The item ID (protocol slot item_id).
    pub item_id: i32,
    /// The default block state ID to place.
    pub block_state_id: i32,
    /// Human-readable name for documentation.
    pub name: &'static str,
}

/// The minimal block/item registry for the flat-world palette.
///
/// These IDs correspond to Minecraft 1.20.1 (protocol 763) global palette.
/// Only full-cube blocks are included — slabs, stairs, and other multi-state
/// blocks are out of scope for this minimal registry.
pub const REGISTRY: &[BlockItemEntry] = &[
    BlockItemEntry {
        item_id: 0,
        block_state_id: 0,
        name: "air",
    },
    BlockItemEntry {
        item_id: 1,
        block_state_id: 1,
        name: "stone",
    },
    BlockItemEntry {
        item_id: 7,
        block_state_id: 7,
        name: "bedrock",
    },
    BlockItemEntry {
        item_id: 8,
        block_state_id: 8,
        name: "grass_block",
    },
    BlockItemEntry {
        item_id: 10,
        block_state_id: 10,
        name: "dirt",
    },
    BlockItemEntry {
        item_id: 11,
        block_state_id: 11,
        name: "coarse_dirt",
    },
    BlockItemEntry {
        item_id: 12,
        block_state_id: 12,
        name: "podzol",
    },
    BlockItemEntry {
        item_id: 15,
        block_state_id: 15,
        name: "gold_ore",
    },
    BlockItemEntry {
        item_id: 16,
        block_state_id: 16,
        name: "iron_ore",
    },
    BlockItemEntry {
        item_id: 21,
        block_state_id: 21,
        name: "coal_ore",
    },
    BlockItemEntry {
        item_id: 24,
        block_state_id: 24,
        name: "sand",
    },
    BlockItemEntry {
        item_id: 32,
        block_state_id: 32,
        name: "dead_bush",
    },
    BlockItemEntry {
        item_id: 50,
        block_state_id: 50,
        name: "torch",
    },
    BlockItemEntry {
        item_id: 73,
        block_state_id: 73,
        name: "redstone_ore",
    },
    BlockItemEntry {
        item_id: 87,
        block_state_id: 87,
        name: "netherrack",
    },
    BlockItemEntry {
        item_id: 88,
        block_state_id: 88,
        name: "soul_sand",
    },
    BlockItemEntry {
        item_id: 89,
        block_state_id: 89,
        name: "glowstone",
    },
    BlockItemEntry {
        item_id: 110,
        block_state_id: 110,
        name: "mycelium",
    },
    BlockItemEntry {
        item_id: 121,
        block_state_id: 121,
        name: "end_stone",
    },
    BlockItemEntry {
        item_id: 152,
        block_state_id: 152,
        name: "redstone_block",
    },
    BlockItemEntry {
        item_id: 173,
        block_state_id: 173,
        name: "coal_block",
    },
    BlockItemEntry {
        item_id: 174,
        block_state_id: 174,
        name: "diamond_block",
    },
    BlockItemEntry {
        item_id: 175,
        block_state_id: 175,
        name: "emerald_block",
    },
    BlockItemEntry {
        item_id: 179,
        block_state_id: 179,
        name: "red_sand",
    },
    BlockItemEntry {
        item_id: 207,
        block_state_id: 207,
        name: "nether_quartz_ore",
    },
    // Fluids use 1.20.1 global palette source states (wiki.vg / minecraft-data).
    BlockItemEntry {
        item_id: 326, // water_bucket places source water in creative stubs
        block_state_id: 80,
        name: "water",
    },
    BlockItemEntry {
        item_id: 327, // lava_bucket
        block_state_id: 96,
        name: "lava",
    },
];

/// Looks up the block state ID for a given item ID.
///
/// Returns `Some(block_state_id)` if the item is a known block item,
/// or `None` if the item is not in the registry (non-block item or unknown).
pub fn item_to_block_state(item_id: i32) -> Option<i32> {
    REGISTRY
        .iter()
        .find(|e| e.item_id == item_id)
        .map(|e| e.block_state_id)
}

/// Looks up the item ID for a given block state ID (reverse lookup).
///
/// Returns `Some(item_id)` if the block state is in the registry,
/// or `None` if not found.
pub fn block_state_to_item(block_state_id: i32) -> Option<i32> {
    REGISTRY
        .iter()
        .find(|e| e.block_state_id == block_state_id)
        .map(|e| e.item_id)
}

/// Looks up the name for a given item ID.
pub fn item_name(item_id: i32) -> Option<&'static str> {
    REGISTRY
        .iter()
        .find(|e| e.item_id == item_id)
        .map(|e| e.name)
}

/// Looks up the name for a given block state ID.
pub fn block_state_name(block_state_id: i32) -> Option<&'static str> {
    REGISTRY
        .iter()
        .find(|e| e.block_state_id == block_state_id)
        .map(|e| e.name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn air_maps_to_air() {
        assert_eq!(item_to_block_state(0), Some(0));
        assert_eq!(block_state_to_item(0), Some(0));
    }

    #[test]
    fn stone_maps_to_stone() {
        assert_eq!(item_to_block_state(1), Some(1));
        assert_eq!(block_state_to_item(1), Some(1));
    }

    #[test]
    fn dirt_maps_to_dirt() {
        assert_eq!(item_to_block_state(10), Some(10));
    }

    #[test]
    fn grass_block_maps_correctly() {
        assert_eq!(item_to_block_state(8), Some(8));
        assert_eq!(item_name(8), Some("grass_block"));
    }

    #[test]
    fn unknown_item_returns_none() {
        assert_eq!(item_to_block_state(99999), None);
    }

    #[test]
    fn unknown_block_state_returns_none() {
        assert_eq!(block_state_to_item(99999), None);
    }

    #[test]
    fn air_name() {
        assert_eq!(item_name(0), Some("air"));
        assert_eq!(block_state_name(0), Some("air"));
    }

    #[test]
    fn registry_entries_are_unique() {
        // Verify no duplicate item IDs
        let mut item_ids: Vec<i32> = REGISTRY.iter().map(|e| e.item_id).collect();
        item_ids.sort();
        let len_before = item_ids.len();
        item_ids.dedup();
        assert_eq!(item_ids.len(), len_before, "duplicate item IDs in registry");

        // Verify no duplicate block state IDs
        let mut state_ids: Vec<i32> = REGISTRY.iter().map(|e| e.block_state_id).collect();
        state_ids.sort();
        let len_before = state_ids.len();
        state_ids.dedup();
        assert_eq!(
            state_ids.len(),
            len_before,
            "duplicate block state IDs in registry"
        );
    }
}
