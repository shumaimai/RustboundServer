//! Flat chunk generator for protocol 763 (1.20.1).
//!
//! Produces valid Chunk Data payloads for a simple flat world so vanilla
//! clients can render terrain. The generator creates a stone plateau from
//! y=-64 through y=63 so players spawning at y=64 have solid ground.
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

/// Block state ID for stone (1.20.1 global palette).
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

/// Builds a minimal valid heightmaps NBT blob for a flat world.
///
/// Contains `MOTION_BLOCKING` and `WORLD_SURFACE` heightmaps. Values are the
/// number of blocks above `min_y` (-64) up to and including the top solid
/// block (y=63 → height 128).
fn build_heightmaps_nbt() -> Vec<u8> {
    let mut buf = Vec::new();

    // Surface at y=63 → 63 - (-64) + 1 = 128 above world bottom.
    const SURFACE_HEIGHT: u32 = 128;
    let packed = pack_heightmap_long(SURFACE_HEIGHT);

    // Root TAG_COMPOUND with empty name
    buf.push(TAG_COMPOUND);
    write_nbt_string(&mut buf, "");

    // MOTION_BLOCKING heightmap (TAG_LONG_ARRAY)
    buf.push(TAG_LONG_ARRAY);
    write_nbt_string(&mut buf, "MOTION_BLOCKING");
    write_nbt_int(&mut buf, HEIGHTMAP_LONGS as i32);
    for _ in 0..HEIGHTMAP_LONGS {
        buf.extend_from_slice(&packed.to_be_bytes());
    }

    // WORLD_SURFACE heightmap (TAG_LONG_ARRAY)
    buf.push(TAG_LONG_ARRAY);
    write_nbt_string(&mut buf, "WORLD_SURFACE");
    write_nbt_int(&mut buf, HEIGHTMAP_LONGS as i32);
    for _ in 0..HEIGHTMAP_LONGS {
        buf.extend_from_slice(&packed.to_be_bytes());
    }

    // End root compound
    buf.push(TAG_END);

    buf
}

/// Packs seven identical 9-bit heightmap samples into one long (1.16+ format).
fn pack_heightmap_long(value: u32) -> i64 {
    let v = i64::from(value & 0x1FF);
    let mut result = 0i64;
    for i in 0..7 {
        result |= v << (9 * i);
    }
    result
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

/// Encodes a single chunk section with a single-valued palette (BPE = 0).
///
/// Per the public chunk format (protocol 763 / 1.20.1), a single-valued
/// palette is `Bits Per Entry = 0`, then a single global-palette `Value`
/// VarInt (no palette length), then a Data Array Length VarInt that is
/// always 0. Indirect palettes (BPE 4–8) are the ones that prefix a length.
fn encode_section(buf: &mut Vec<u8>, block_count: i16, block_state: i32, biome_id: i32) {
    // Block count (Short) — non-air blocks in the section
    encode_i16(block_count, buf);

    // Block states (single-valued)
    encode_u8(0, buf); // bits per block = 0
    encode_var_int(block_state, buf); // Value (no length prefix)
    encode_var_int(0, buf); // data array length = 0

    // Biomes (single-valued)
    encode_u8(0, buf); // bits per biome = 0
    encode_var_int(biome_id, buf); // Value (no length prefix)
    encode_var_int(0, buf); // data array length = 0
}

/// Generates the chunk section data for a flat world.
///
/// Solid stone fills sections 0..=7 (world y=-64..=63). Everything above is
/// air, so players spawning at y=64 stand on the flat surface.
///
/// Section count is NOT encoded: the client derives it from the dimension
/// height (384 / 16 = 24 for overworld).
fn build_section_data() -> Vec<u8> {
    let mut buf = Vec::new();

    // Section 7 top is y=63; spawn at y=64 stands on that surface.
    const TOP_SOLID_SECTION: usize = 7;

    for section_idx in 0..NUM_SECTIONS {
        if section_idx <= TOP_SOLID_SECTION {
            encode_section(
                &mut buf,
                SECTION_BLOCK_COUNT as i16,
                STONE_BLOCK_STATE,
                PLAINS_BIOME_ID,
            );
        } else {
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
        // Each single-valued section is: i16 + 2 × (u8 + VarInt value + VarInt data_len)
        // block: 2 + 1 + 1 + 1 = 5; biome: 1 + 1 + 1 = 3; total 8 bytes
        assert_eq!(data.len(), NUM_SECTIONS * 8);
        assert!(!data.is_empty());
    }

    #[test]
    fn single_valued_section_wire_has_no_palette_length() {
        // Stone section: block_count=4096 (0x10_00 BE), BPE=0, value=1, data_len=0,
        // biome BPE=0, value=0, data_len=0.
        let mut stone = Vec::new();
        encode_section(&mut stone, 4096, STONE_BLOCK_STATE, PLAINS_BIOME_ID);
        assert_eq!(
            stone,
            vec![0x10, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00],
            "single-valued stone section must omit palette length"
        );

        let mut air = Vec::new();
        encode_section(&mut air, 0, AIR_BLOCK_STATE, PLAINS_BIOME_ID);
        assert_eq!(
            air,
            vec![0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
            "single-valued air section must omit palette length"
        );

        let data = build_section_data();
        assert_eq!(&data[..8], stone.as_slice());
        assert_eq!(&data[8 * 7..8 * 8], stone.as_slice());
        assert_eq!(&data[8 * 8..8 * 9], air.as_slice());
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
