//! Flat/void chunk generator for protocol 763 (1.20.1).
//!
//! Produces valid Chunk Data payloads for a simple flat world so vanilla
//! clients can render terrain. The generator creates a void world with a
//! single stone platform at y=-64 (the bottom of the world).
//!
//! The chunk section format follows public protocol documentation:
//! each section has a block-count (Short), block states (palette + data),
//! and biomes (palette + data).

use rustbound_protocol::primitives::{encode_i16, encode_u8, encode_var_int};

// Re-export for convenience
pub use rustbound_protocol::primitives::encode_i16 as _encode_i16;

/// Number of sections in a 1.20.1 world (384 blocks / 16 = 24 sections).
pub const NUM_SECTIONS: usize = 24;

/// Number of blocks per section side (16x16 = 256).
pub const SECTION_BLOCK_COUNT: usize = 4096;

/// Block state ID for air.
const AIR_BLOCK_STATE: i32 = 0;

/// Block state ID for stone (approximate global palette ID).
const STONE_BLOCK_STATE: i32 = 1;

/// Biome ID for plains (matches registry codec fixture).
const PLAINS_BIOME_ID: i32 = 0;

/// NBT tag type for TAG_LONG_ARRAY.
const TAG_LONG_ARRAY: u8 = 12;

/// NBT tag type for TAG_COMPOUND.
const TAG_COMPOUND: u8 = 10;

/// NBT tag type for TAG_END.
const TAG_END: u8 = 0;

/// Number of longs in a 16x16 heightmap with 9-bit entries.
/// 256 entries * 9 bits = 2304 bits / 64 = 36 longs.
const HEIGHTMAP_LONGS: usize = 36;

/// Builds a minimal valid heightmaps NBT blob for a void/flat world.
///
/// Contains `MOTION_BLOCKING` and `WORLD_SURFACE` heightmaps with all
/// values set to 0 (indicating the lowest possible height).
fn build_heightmaps_nbt() -> Vec<u8> {
    let mut buf = Vec::new();

    // Root TAG_COMPOUND with empty name
    buf.push(TAG_COMPOUND);
    write_nbt_string(&mut buf, "");

    // MOTION_BLOCKING heightmap (TAG_LONG_ARRAY)
    buf.push(TAG_LONG_ARRAY);
    write_nbt_string(&mut buf, "MOTION_BLOCKING");
    write_nbt_int(&mut buf, HEIGHTMAP_LONGS as i32);
    for _ in 0..HEIGHTMAP_LONGS {
        buf.extend_from_slice(&0i64.to_be_bytes());
    }

    // WORLD_SURFACE heightmap (TAG_LONG_ARRAY)
    buf.push(TAG_LONG_ARRAY);
    write_nbt_string(&mut buf, "WORLD_SURFACE");
    write_nbt_int(&mut buf, HEIGHTMAP_LONGS as i32);
    for _ in 0..HEIGHTMAP_LONGS {
        buf.extend_from_slice(&0i64.to_be_bytes());
    }

    // End root compound
    buf.push(TAG_END);

    buf
}

fn write_nbt_string(buf: &mut Vec<u8>, value: &str) {
    let bytes = value.as_bytes();
    let len = bytes.len() as u16;
    buf.extend_from_slice(&len.to_be_bytes());
    buf.extend_from_slice(bytes);
}

fn write_nbt_int(buf: &mut Vec<u8>, value: i32) {
    buf.extend_from_slice(&value.to_be_bytes());
}

/// Encodes a single chunk section with a single-value block palette.
///
/// When `bits_per_block == 0`, the palette has exactly one entry and the
/// data array is empty (length 0).
fn encode_section(buf: &mut Vec<u8>, block_count: i16, block_state: i32, biome_id: i32) {
    // Block count (Short)
    encode_i16(block_count, buf);

    // Block states
    encode_u8(0, buf); // bits per block = 0 (single value palette)
    encode_var_int(1, buf); // palette length = 1
    encode_var_int(block_state, buf); // palette entry
    encode_var_int(0, buf); // data array length = 0

    // Biomes
    encode_u8(0, buf); // bits per biome = 0 (single value palette)
    encode_var_int(1, buf); // palette length = 1
    encode_var_int(biome_id, buf); // palette entry
    encode_var_int(0, buf); // data array length = 0
}

/// Generates the chunk section data for a flat world.
///
/// The flat world has a single layer of stone at the bottom (y=-64)
/// and air everywhere else. Section 0 (y=-64 to y=-49) contains the
/// stone layer; all other sections are empty (air).
fn build_section_data() -> Vec<u8> {
    let mut buf = Vec::new();

    // Number of sections (VarInt)
    encode_var_int(NUM_SECTIONS as i32, &mut buf);

    for section_idx in 0..NUM_SECTIONS {
        if section_idx == 0 {
            // Bottom section: stone at y=-64 (all 4096 blocks are stone)
            encode_section(
                &mut buf,
                SECTION_BLOCK_COUNT as i16,
                STONE_BLOCK_STATE,
                PLAINS_BIOME_ID,
            );
        } else {
            // All other sections: air
            encode_section(&mut buf, 0, AIR_BLOCK_STATE, PLAINS_BIOME_ID);
        }
    }

    buf
}

/// Generates a complete chunk data payload for the given chunk position.
///
/// Returns a tuple of (heightmaps_nbt, section_data, block_entities).
/// Block entities is empty for a flat world.
pub fn generate_chunk(_chunk_x: i32, _chunk_z: i32) -> (Vec<u8>, Vec<u8>, Vec<Vec<u8>>) {
    let heightmaps = build_heightmaps_nbt();
    let data = build_section_data();
    let block_entities: Vec<Vec<u8>> = Vec::new();
    (heightmaps, data, block_entities)
}

/// Builds a `ChunkData` packet ready for encoding.
pub fn build_chunk_data_packet(chunk_x: i32, chunk_z: i32) -> rustbound_protocol::play::ChunkData {
    let (heightmaps, data, block_entities) = generate_chunk(chunk_x, chunk_z);
    rustbound_protocol::play::ChunkData {
        chunk_x,
        chunk_z,
        heightmaps,
        data,
        block_entities,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustbound_protocol::play::{
        PlayDecodeOutcome, PlayPacket, decode_chunk_data, encode_chunk_data,
    };

    const TEST_MAX_FRAME: usize = 65536 * 4;

    #[test]
    fn heightmaps_nbt_is_valid() {
        let nbt = build_heightmaps_nbt();
        // Root should be TAG_COMPOUND
        assert_eq!(nbt[0], TAG_COMPOUND);
        // Should contain MOTION_BLOCKING and WORLD_SURFACE
        let nbt_str = String::from_utf8_lossy(&nbt);
        assert!(nbt_str.contains("MOTION_BLOCKING"));
        assert!(nbt_str.contains("WORLD_SURFACE"));
    }

    #[test]
    fn section_data_has_correct_section_count() -> Result<(), Box<dyn std::error::Error>> {
        let data = build_section_data();
        // First bytes should be a VarInt encoding 24
        let mut input = data.as_slice();
        let count = rustbound_protocol::primitives::decode_var_int(&mut input)?;
        assert_eq!(count, NUM_SECTIONS as i32);
        Ok(())
    }

    #[test]
    fn chunk_data_roundtrip() -> Result<(), Box<dyn std::error::Error>> {
        let packet = build_chunk_data_packet(10, -5);
        let mut wire = Vec::new();
        encode_chunk_data(&packet, TEST_MAX_FRAME, &mut wire)?;

        let mut input = wire.as_slice();
        match decode_chunk_data(&mut input, TEST_MAX_FRAME)? {
            PlayDecodeOutcome::Complete(PlayPacket::ChunkData(decoded)) => {
                assert_eq!(decoded.chunk_x, 10);
                assert_eq!(decoded.chunk_z, -5);
                assert!(!decoded.heightmaps.is_empty());
                assert!(!decoded.data.is_empty());
            }
            _ => panic!("expected complete ChunkData"),
        }
        assert!(input.is_empty());
        Ok(())
    }

    #[test]
    fn chunk_generation_is_deterministic() {
        let (h1, d1, _) = generate_chunk(0, 0);
        let (h2, d2, _) = generate_chunk(0, 0);
        assert_eq!(h1, h2);
        assert_eq!(d1, d2);
    }

    #[test]
    fn different_chunks_have_same_data() {
        // Flat world: all chunks have the same section data
        let (_, d1, _) = generate_chunk(0, 0);
        let (_, d2, _) = generate_chunk(100, -200);
        assert_eq!(d1, d2);
    }
}
