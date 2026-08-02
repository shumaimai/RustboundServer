//! Minimal registry codec NBT blob for Join Game (protocol 763).
//!
//! The registry codec is sent as part of the Join Game packet and contains
//! dimension type, biome, and chat type registries. This module provides a
//! minimal valid NBT blob that a vanilla 1.20.1 client can parse.
//!
//! The blob is constructed programmatically using a simple NBT builder
//! rather than copied from any proprietary source. The structure follows
//! public protocol documentation (wiki.vg / Minecraft Wiki).

// NBT tag type constants
const TAG_END: u8 = 0;
const TAG_BYTE: u8 = 1;
const TAG_INT: u8 = 3;
const TAG_FLOAT: u8 = 5;
const TAG_STRING: u8 = 8;
const TAG_LIST: u8 = 9;
const TAG_COMPOUND: u8 = 10;

/// A simple NBT writer that builds the binary NBT format.
struct NbtWriter {
    buf: Vec<u8>,
}

impl NbtWriter {
    fn new() -> Self {
        Self { buf: Vec::new() }
    }

    fn write_i32(&mut self, value: i32) {
        self.buf.extend_from_slice(&value.to_be_bytes());
    }

    fn write_f32(&mut self, value: f32) {
        self.buf.extend_from_slice(&value.to_be_bytes());
    }

    fn write_string(&mut self, value: &str) {
        let bytes = value.as_bytes();
        let len = bytes.len() as u16;
        self.buf.extend_from_slice(&len.to_be_bytes());
        self.buf.extend_from_slice(bytes);
    }

    fn write_named_byte(&mut self, name: &str, value: u8) {
        self.buf.push(TAG_BYTE);
        self.write_string(name);
        self.buf.push(value);
    }

    fn write_named_int(&mut self, name: &str, value: i32) {
        self.buf.push(TAG_INT);
        self.write_string(name);
        self.write_i32(value);
    }

    fn write_named_float(&mut self, name: &str, value: f32) {
        self.buf.push(TAG_FLOAT);
        self.write_string(name);
        self.write_f32(value);
    }

    fn write_named_string(&mut self, name: &str, value: &str) {
        self.buf.push(TAG_STRING);
        self.write_string(name);
        self.write_string(value);
    }

    fn begin_named_compound(&mut self, name: &str) {
        self.buf.push(TAG_COMPOUND);
        self.write_string(name);
    }

    fn end_compound(&mut self) {
        self.buf.push(TAG_END);
    }

    fn begin_named_list(&mut self, name: &str, element_type: u8, length: i32) {
        self.buf.push(TAG_LIST);
        self.write_string(name);
        self.buf.push(element_type);
        self.write_i32(length);
    }

    /// Begins the root compound (unnamed in the packet context, but NBT
    /// requires a name — use empty string).
    fn begin_root(&mut self) {
        self.buf.push(TAG_COMPOUND);
        self.write_string("");
    }

    fn finish(self) -> Vec<u8> {
        self.buf
    }
}

/// Builds a minimal valid dimension type element compound (unnamed, inside a list).
fn write_dimension_type_element(w: &mut NbtWriter) {
    // TAG_COMPOUND element (no name inside list)
    w.write_named_byte("has_skylight", 1);
    w.write_named_byte("has_ceiling", 0);
    w.write_named_byte("ultrawarm", 0);
    w.write_named_byte("natural", 1);
    w.write_named_float("coordinate_scale", 1.0);
    w.write_named_byte("bed_works", 1);
    w.write_named_string("effects", "minecraft:overworld");
    w.write_named_float("ambient_light", 0.0);
    w.write_named_byte("respawn_anchor_works", 0);
    w.write_named_int("min_y", -64);
    w.write_named_int("height", 384);
    w.write_named_int("logical_height", 384);
    w.write_named_string("infiniburn", "minecraft:infiniburn_overworld");
    w.write_named_byte("piglin_safe", 0);
    w.write_named_byte("has_raids", 1);
    w.write_named_int("monster_spawn_block_light_limit", 0);
    w.end_compound();
}

/// Builds a minimal valid biome element compound (unnamed, inside a list).
fn write_biome_element(w: &mut NbtWriter) {
    w.write_named_string("precipitation", "rain");
    w.write_named_float("temperature", 0.8);
    w.write_named_float("downfall", 0.4);
    w.begin_named_compound("effects");
    w.write_named_int("sky_color", 7907327);
    w.write_named_int("water_color", 4159204);
    w.write_named_int("water_fog_color", 329011);
    w.write_named_int("fog_color", 12638463);
    w.end_compound();
    w.end_compound();
}

/// Builds a minimal valid chat type element compound (unnamed, inside a list).
fn write_chat_type_element(w: &mut NbtWriter) {
    w.begin_named_compound("chat");
    w.write_named_string("decoration", "minecraft:system");
    w.end_compound();
    w.end_compound();
}

/// Builds the full registry codec NBT blob for Join Game.
///
/// Contains three registries required by vanilla 1.20.1:
/// - `minecraft:dimension_type` with `minecraft:overworld`
/// - `minecraft:worldgen/biome` with `minecraft:plains`
/// - `minecraft:chat_type` with `minecraft:system`
pub fn build_registry_codec() -> Vec<u8> {
    let mut w = NbtWriter::new();

    // Root compound
    w.begin_root();

    // --- minecraft:dimension_type registry ---
    w.begin_named_compound("minecraft:dimension_type");
    w.write_named_string("type", "minecraft:dimension_type");
    w.begin_named_list("value", TAG_COMPOUND, 1);
    // Entry: minecraft:overworld
    {
        // Each list element is a compound (no name/tag-type prefix inside list)
        w.write_named_string("name", "minecraft:overworld");
        w.write_named_int("id", 0);
        w.begin_named_compound("element");
        write_dimension_type_element(&mut w);
        w.end_compound(); // end entry compound
    }
    w.end_compound(); // end minecraft:dimension_type

    // --- minecraft:worldgen/biome registry ---
    w.begin_named_compound("minecraft:worldgen/biome");
    w.write_named_string("type", "minecraft:worldgen/biome");
    w.begin_named_list("value", TAG_COMPOUND, 1);
    {
        w.write_named_string("name", "minecraft:plains");
        w.write_named_int("id", 0);
        w.begin_named_compound("element");
        write_biome_element(&mut w);
        w.end_compound();
    }
    w.end_compound();

    // --- minecraft:chat_type registry ---
    w.begin_named_compound("minecraft:chat_type");
    w.write_named_string("type", "minecraft:chat_type");
    w.begin_named_list("value", TAG_COMPOUND, 1);
    {
        w.write_named_string("name", "minecraft:system");
        w.write_named_int("id", 0);
        w.begin_named_compound("element");
        write_chat_type_element(&mut w);
        w.end_compound();
    }
    w.end_compound();

    // End root compound
    w.end_compound();

    w.finish()
}

#[cfg(test)]
mod tests {
    use super::build_registry_codec;

    #[test]
    fn registry_codec_is_non_empty() {
        let blob = build_registry_codec();
        assert!(!blob.is_empty(), "registry codec blob should not be empty");
        // Root tag should be a TAG_COMPOUND (10)
        assert_eq!(blob[0], 10);
    }

    #[test]
    fn registry_codec_contains_dimension_type() {
        let blob = build_registry_codec();
        let blob_str = String::from_utf8_lossy(&blob);
        assert!(
            blob_str.contains("minecraft:dimension_type"),
            "registry codec should contain dimension_type registry"
        );
        assert!(
            blob_str.contains("minecraft:overworld"),
            "registry codec should contain overworld dimension"
        );
    }

    #[test]
    fn registry_codec_contains_biome() {
        let blob = build_registry_codec();
        let blob_str = String::from_utf8_lossy(&blob);
        assert!(
            blob_str.contains("minecraft:worldgen/biome"),
            "registry codec should contain biome registry"
        );
        assert!(
            blob_str.contains("minecraft:plains"),
            "registry codec should contain plains biome"
        );
    }

    #[test]
    fn registry_codec_contains_chat_type() {
        let blob = build_registry_codec();
        let blob_str = String::from_utf8_lossy(&blob);
        assert!(
            blob_str.contains("minecraft:chat_type"),
            "registry codec should contain chat_type registry"
        );
    }

    #[test]
    fn registry_codec_is_deterministic() {
        let blob1 = build_registry_codec();
        let blob2 = build_registry_codec();
        assert_eq!(blob1, blob2, "registry codec should be deterministic");
    }
}
