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

/// Number of longs in a 16×16 heightmap with 9-bit entries for height 384.
///
/// Bits per entry = ceil(log2(world_height + 1)) = ceil(log2(385)) = 9.
/// Since 1.16, values do not span longs: each long holds floor(64/9)=7 entries,
/// so 256 entries require ceil(256/7) = 37 longs.
const HEIGHTMAP_LONGS: usize = 37;

/// Number of light sections in a 1.20.1 world.
/// The world is 384 blocks tall (-64 to 319), which is 24 sections.
/// Light data includes one extra section above and below, so 26 total.
const NUM_LIGHT_SECTIONS: usize = 26;

/// Size of a single light array in bytes (2048 bytes = 16384 nibbles for
/// 16x16x16 blocks).
const LIGHT_ARRAY_SIZE: usize = 2048;

/// Builds a BitSet as encoded in protocol 763: VarInt length followed by
/// that many i64 longs (big-endian).
fn encode_bitset(buf: &mut Vec<u8>, longs: &[i64]) {
    encode_var_int(longs.len() as i32, buf);
    for &val in longs {
        buf.extend_from_slice(&val.to_be_bytes());
    }
}

/// Builds the light data blob for a flat/void world.
///
/// For a flat world with no obstructions, all sections have full skylight
/// (level 15). Block light is zero everywhere (no light sources).
///
/// The light data format (protocol 763 / 1.20+) is:
/// 1. SkyLightMask (BitSet)
/// 2. BlockLightMask (BitSet)
/// 3. EmptySkyLightMask (BitSet)
/// 4. EmptyBlockLightMask (BitSet)
/// 5. SkyLight arrays (VarInt count + each Prefixed Array of 2048 bytes)
/// 6. BlockLight arrays (VarInt count + each Prefixed Array of 2048 bytes)
///
/// Note: Trust Edges was removed in 1.20; do not prefix a bool here.
///
/// We set SkyLightMask to cover all 26 light sections (bits 0..25 = 0x3FFFFFF),
/// provide 26 full skylight arrays (all 0xFF = level 15), and leave
/// BlockLightMask empty (no block light data).
fn build_light_data() -> Vec<u8> {
    let mut buf = Vec::new();

    // SkyLightMask: bits 0..25 set = all 26 light sections have skylight data
    // 0x3FFFFFF = 0b00000000000000111111111111111111111111 (26 bits)
    let sky_mask: i64 = 0x03FF_FFFF;
    encode_bitset(&mut buf, &[sky_mask]);

    // BlockLightMask: no block light data (all zeros)
    encode_bitset(&mut buf, &[]);

    // EmptySkyLightMask: no empty skylight sections (we provide data for all)
    encode_bitset(&mut buf, &[]);

    // EmptyBlockLightMask: all sections are empty block light (all zeros)
    // This tells the client block light = 0 for all sections
    encode_bitset(&mut buf, &[sky_mask]);

    // SkyLight arrays: 26 arrays, each a Prefixed Array of 2048 bytes of 0xFF
    encode_var_int(NUM_LIGHT_SECTIONS as i32, &mut buf);
    for _ in 0..NUM_LIGHT_SECTIONS {
        encode_var_int(LIGHT_ARRAY_SIZE as i32, &mut buf);
        // 0xFF = every nibble is 15 (full skylight)
        buf.extend_from_slice(&[0xFF; LIGHT_ARRAY_SIZE]);
    }

    // BlockLight arrays: 0 arrays (no block light data)
    encode_var_int(0, &mut buf);

    buf
}

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
///
/// Section count is NOT encoded: the client derives it from the dimension
/// height (384 / 16 = 24 for overworld).
fn build_section_data() -> Vec<u8> {
    let mut buf = Vec::new();

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
/// Returns a tuple of (heightmaps_nbt, section_data, block_entities, light_data).
/// Block entities is empty for a flat world.
pub fn generate_chunk(_chunk_x: i32, _chunk_z: i32) -> (Vec<u8>, Vec<u8>, Vec<Vec<u8>>, Vec<u8>) {
    let heightmaps = build_heightmaps_nbt();
    let data = build_section_data();
    let block_entities: Vec<Vec<u8>> = Vec::new();
    let light_data = build_light_data();
    (heightmaps, data, block_entities, light_data)
}

/// Builds a `ChunkData` packet ready for encoding.
pub fn build_chunk_data_packet(chunk_x: i32, chunk_z: i32) -> rustbound_protocol::play::ChunkData {
    let (heightmaps, data, block_entities, light_data) = generate_chunk(chunk_x, chunk_z);
    rustbound_protocol::play::ChunkData {
        chunk_x,
        chunk_z,
        heightmaps,
        data,
        block_entities,
        light_data,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustbound_protocol::play::{
        PlayDecodeOutcome, PlayPacket, decode_chunk_data, encode_chunk_data,
    };
    use std::io::Read;

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
    fn section_data_has_expected_size() {
        let data = build_section_data();
        // Each single-value section is: i16 + (u8 + varint*3)*2
        // block: 2 + 1 + 1 + 1 + 1 = 6; biome: 1 + 1 + 1 + 1 = 4; total 10 bytes
        // (palette varints for length=1 and value=0/1 and data_len=0 are 1 byte each)
        assert_eq!(data.len(), NUM_SECTIONS * 10);
        assert!(!data.is_empty());
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
                assert!(
                    !decoded.light_data.is_empty(),
                    "light data should not be empty"
                );
            }
            _ => panic!("expected complete ChunkData"),
        }
        assert!(input.is_empty());
        Ok(())
    }

    #[test]
    fn light_data_is_valid_format() -> Result<(), Box<dyn std::error::Error>> {
        let light = build_light_data();
        // Should be non-empty
        assert!(!light.is_empty());

        // Decode the light data to verify structure
        let mut input = light.as_slice();
        // SkyLightMask: VarInt count + longs
        let sky_mask_count = rustbound_protocol::primitives::decode_var_int(&mut input)?;
        assert_eq!(sky_mask_count, 1, "sky mask should have 1 long");
        let _sky_mask = rustbound_protocol::primitives::decode_i64(&mut input)?;

        // BlockLightMask: 0 longs
        let block_mask_count = rustbound_protocol::primitives::decode_var_int(&mut input)?;
        assert_eq!(block_mask_count, 0, "block light mask should be empty");

        // EmptySkyLightMask: 0 longs
        let empty_sky_count = rustbound_protocol::primitives::decode_var_int(&mut input)?;
        assert_eq!(empty_sky_count, 0, "empty sky mask should be empty");

        // EmptyBlockLightMask: 1 long
        let empty_block_count = rustbound_protocol::primitives::decode_var_int(&mut input)?;
        assert_eq!(empty_block_count, 1, "empty block mask should have 1 long");
        let _empty_block = rustbound_protocol::primitives::decode_i64(&mut input)?;

        // SkyLight arrays: 26 Prefixed Arrays of 2048 bytes each
        let sky_array_count = rustbound_protocol::primitives::decode_var_int(&mut input)?;
        assert_eq!(
            sky_array_count, NUM_LIGHT_SECTIONS as i32,
            "should have {NUM_LIGHT_SECTIONS} sky light arrays"
        );
        for _ in 0..NUM_LIGHT_SECTIONS {
            let array_len = rustbound_protocol::primitives::decode_var_int(&mut input)?;
            assert_eq!(array_len, LIGHT_ARRAY_SIZE as i32);
            let mut array = [0u8; LIGHT_ARRAY_SIZE];
            input.read_exact(&mut array)?;
            assert!(array.iter().all(|&b| b == 0xFF), "skylight should be 0xFF");
        }

        // BlockLight arrays: 0
        let block_array_count = rustbound_protocol::primitives::decode_var_int(&mut input)?;
        assert_eq!(block_array_count, 0, "should have 0 block light arrays");

        assert!(input.is_empty(), "no trailing bytes in light data");
        Ok(())
    }

    #[test]
    fn chunk_generation_is_deterministic() {
        let (h1, d1, _, l1) = generate_chunk(0, 0);
        let (h2, d2, _, l2) = generate_chunk(0, 0);
        assert_eq!(h1, h2);
        assert_eq!(d1, d2);
        assert_eq!(l1, l2);
    }

    #[test]
    fn different_chunks_have_same_data() {
        // Flat world: all chunks have the same section data
        let (_, d1, _, _) = generate_chunk(0, 0);
        let (_, d2, _, _) = generate_chunk(100, -200);
        assert_eq!(d1, d2);
    }
}
