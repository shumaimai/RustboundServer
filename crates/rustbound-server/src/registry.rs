//! Minimal block/item ID registry for hakoniwa placement.
//!
//! Maps protocol item IDs → 1.20.1 global palette block state IDs for creative
//! placement and dig drops. IDs come from public minecraft-data / wiki.vg
//! (clean-room; no jars).

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

/// Minimal registry used by creative place / pack tooling.
///
/// First match wins for `item_to_block_state`. Prefer real 1.20.1 item IDs;
/// keep a few legacy stubs that in-tree tests still use.
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
    // Real IDs
    BlockItemEntry {
        item_id: 43,
        block_state_id: 79,
        name: "bedrock",
    },
    BlockItemEntry {
        item_id: 14,
        block_state_id: 9,
        name: "grass_block",
    },
    BlockItemEntry {
        item_id: 15,
        block_state_id: 10,
        name: "dirt",
    },
    BlockItemEntry {
        item_id: 23,
        block_state_id: 15,
        name: "oak_planks",
    },
    BlockItemEntry {
        item_id: 44,
        block_state_id: 112,
        name: "sand",
    },
    BlockItemEntry {
        item_id: 303,
        block_state_id: 5850,
        name: "netherrack",
    },
    BlockItemEntry {
        item_id: 304,
        block_state_id: 5851,
        name: "soul_sand",
    },
    BlockItemEntry {
        item_id: 310,
        block_state_id: 5864,
        name: "glowstone",
    },
    BlockItemEntry {
        item_id: 355,
        block_state_id: 7415,
        name: "end_stone",
    },
    BlockItemEntry {
        item_id: 68,
        block_state_id: 10604,
        name: "coal_block",
    },
    BlockItemEntry {
        item_id: 77,
        block_state_id: 4276,
        name: "diamond_block",
    },
    BlockItemEntry {
        item_id: 360,
        block_state_id: 7665,
        name: "emerald_block",
    },
    BlockItemEntry {
        item_id: 869,
        block_state_id: 80,
        name: "water",
    },
    BlockItemEntry {
        item_id: 870,
        block_state_id: 96,
        name: "lava",
    },
    BlockItemEntry {
        item_id: 277,
        block_state_id: 2955,
        name: "chest",
    },
    // Legacy creative hotbar stubs (tests / older configs).
    BlockItemEntry {
        item_id: 7,
        block_state_id: 79,
        name: "bedrock_stub",
    },
    BlockItemEntry {
        item_id: 8,
        block_state_id: 9,
        name: "grass_block_stub",
    },
    BlockItemEntry {
        item_id: 10,
        block_state_id: 10,
        name: "dirt_stub",
    },
    BlockItemEntry {
        item_id: 11,
        block_state_id: 11,
        name: "coarse_dirt",
    },
    BlockItemEntry {
        item_id: 24,
        block_state_id: 112,
        name: "sand_stub",
    },
    BlockItemEntry {
        item_id: 326,
        block_state_id: 80,
        name: "water_legacy",
    },
    BlockItemEntry {
        item_id: 327,
        block_state_id: 96,
        name: "lava_legacy",
    },
];

/// Returns `Some(block_state_id)` if the item is a known block item.
pub fn item_to_block_state(item_id: i32) -> Option<i32> {
    REGISTRY
        .iter()
        .find(|e| e.item_id == item_id)
        .map(|e| e.block_state_id)
}

/// Reverse lookup: first matching registry entry for this block state.
pub fn block_state_to_item(block_state_id: i32) -> Option<i32> {
    REGISTRY
        .iter()
        .find(|e| e.block_state_id == block_state_id)
        .map(|e| e.item_id)
}

/// Human-readable name for an item ID, if known.
pub fn item_name(item_id: i32) -> Option<&'static str> {
    REGISTRY
        .iter()
        .find(|e| e.item_id == item_id)
        .map(|e| e.name)
}

/// Human-readable name for a block state ID, if known.
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
    fn air_and_stone_roundtrip() {
        assert_eq!(item_to_block_state(0), Some(0));
        assert_eq!(block_state_to_item(0), Some(0));
        assert_eq!(item_to_block_state(1), Some(1));
        assert_eq!(block_state_to_item(1), Some(1));
    }

    #[test]
    fn dirt_stub_and_real_item_place_dirt() {
        assert_eq!(item_to_block_state(10), Some(10));
        assert_eq!(item_to_block_state(15), Some(10));
    }

    #[test]
    fn nether_end_materials_use_real_palette() {
        assert_eq!(item_to_block_state(303), Some(5850));
        assert_eq!(item_to_block_state(310), Some(5864));
        assert_eq!(item_to_block_state(355), Some(7415));
        assert_ne!(item_to_block_state(303), Some(87));
        assert_ne!(item_to_block_state(355), Some(121));
    }

    #[test]
    fn sand_is_not_bamboo_mosaic() {
        assert_eq!(item_to_block_state(44), Some(112));
        assert_eq!(item_to_block_state(24), Some(112));
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
        assert_eq!(block_state_name(0), Some("air"));
    }

    #[test]
    fn item_ids_are_unique() {
        let mut ids: Vec<i32> = REGISTRY.iter().map(|e| e.item_id).collect();
        let before = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), before, "duplicate item IDs in registry");
    }
}
