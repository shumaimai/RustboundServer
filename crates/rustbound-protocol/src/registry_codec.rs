//! Registry codec NBT blob for Join Game (protocol 763 / 1.20.1).
//!
//! The blob is a fixture generated from the publicly published PrismarineJS
//! `minecraft-data` PC 1.20 `loginPacket.dimensionCodec` (same protocol
//! version family as 1.20.1 / PVN 763). It is not taken from Mojang jars or
//! decompiled sources.

const TAG_END: u8 = 0;
const TAG_COMPOUND: u8 = 10;

/// Full Join Game registry codec for vanilla 1.20.1 clients.
///
/// Sourced from PrismarineJS minecraft-data `data/pc/1.20/loginPacket.json`
/// (`dimensionCodec`), serialized to classic named-root binary NBT
/// (`TAG_Compound` + empty name + payload) as used by 1.20.1 `NbtIo.write`.
pub fn build_registry_codec() -> Vec<u8> {
    include_bytes!("fixtures/registry_codec_1_20_1.nbt").to_vec()
}

/// Minimal empty root compound NBT (`TAG_Compound` with empty name + `TAG_End`).
pub fn empty_nbt_compound() -> Vec<u8> {
    vec![TAG_COMPOUND, 0x00, 0x00, TAG_END]
}

#[cfg(test)]
mod tests {
    use super::{build_registry_codec, empty_nbt_compound};

    #[test]
    fn registry_codec_is_non_empty() {
        let blob = build_registry_codec();
        assert!(!blob.is_empty());
        assert_eq!(blob[0], 10);
    }

    #[test]
    fn registry_codec_contains_required_registries() {
        let blob = build_registry_codec();
        let blob_str = String::from_utf8_lossy(&blob);
        for needle in [
            "minecraft:dimension_type",
            "minecraft:overworld",
            "minecraft:worldgen/biome",
            "minecraft:plains",
            "minecraft:chat_type",
            "minecraft:damage_type",
            "minecraft:trim_pattern",
            "minecraft:trim_material",
            "monster_spawn_light_level",
            "#minecraft:infiniburn_overworld",
        ] {
            assert!(blob_str.contains(needle), "missing {needle}");
        }
    }

    #[test]
    fn registry_codec_is_deterministic() {
        assert_eq!(build_registry_codec(), build_registry_codec());
    }

    #[test]
    fn registry_codec_has_named_root_compound() {
        let blob = build_registry_codec();
        assert_eq!(blob[0], 10);
        assert_eq!(&blob[1..3], &[0, 0]);
    }

    #[test]
    fn empty_nbt_compound_is_self_delimiting() {
        assert_eq!(empty_nbt_compound(), vec![10, 0, 0, 0]);
    }
}
