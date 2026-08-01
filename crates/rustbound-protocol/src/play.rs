//! Play state packet codecs for protocol 763 (Minecraft Java Edition 1.20.1).
//!
//! This module implements the Join Game (clientbound Play `0x01`) packet,
//! which is the first packet sent after Login Success and transitions the
//! client into the Play state.

use std::fmt;

use crate::framing::{DecodeOutcome, FramingError, decode_frame, encode_frame};
use crate::primitives::{
    CodecError, MAX_CHAT_COMPONENT_LENGTH, decode_bool, decode_byte_array, decode_i8, decode_i32,
    decode_i64, decode_string, decode_u8, decode_var_int, encode_bool, encode_byte_array,
    encode_i8, encode_i32, encode_i64, encode_string, encode_u8, encode_var_int,
};
use crate::state::ProtocolState;

/// Packet ID for the clientbound Join Game packet (Login in play).
pub const JOIN_GAME_PACKET_ID: i32 = 0x28;

/// Packet ID for the clientbound Keep Alive (Play) packet.
pub const KEEP_ALIVE_CLIENTBOUND_PACKET_ID: i32 = 0x23;

/// Packet ID for the serverbound Keep Alive (Play) packet.
pub const KEEP_ALIVE_SERVERBOUND_PACKET_ID: i32 = 0x12;

/// Packet ID for the serverbound Set Player Position packet.
pub const SET_PLAYER_POSITION_PACKET_ID: i32 = 0x14;

/// Packet ID for the serverbound Set Player Position and Rotation packet.
pub const SET_PLAYER_POSITION_AND_ROTATION_PACKET_ID: i32 = 0x15;

/// Packet ID for the serverbound Set Player Rotation packet.
pub const SET_PLAYER_ROTATION_PACKET_ID: i32 = 0x16;

/// Packet ID for the clientbound Synchronize Player Position packet.
pub const SYNCHRONIZE_PLAYER_POSITION_PACKET_ID: i32 = 0x3c;

/// Packet ID for the clientbound Disconnect (Play) packet.
pub const DISCONNECT_PLAY_PACKET_ID: i32 = 0x1a;

/// Packet ID for the clientbound Chunk Data and Update Light packet.
pub const CHUNK_DATA_PACKET_ID: i32 = 0x24;

/// Maximum length of a chunk data blob.
pub const MAX_CHUNK_DATA_SIZE: usize = 1048576;

/// Maximum number of block entities in a chunk.
pub const MAX_BLOCK_ENTITIES: usize = 1024;

/// Maximum number of dimension names in a Join Game packet.
pub const MAX_DIMENSION_NAMES: usize = 256;

/// Maximum length of a dimension name or type identifier string.
pub const MAX_IDENTIFIER_LENGTH: usize = 32767;

/// Maximum size of the registry codec NBT blob.
pub const MAX_REGISTRY_CODEC_SIZE: usize = 1048576;

/// An error encountered while encoding or decoding a play packet.
#[derive(Debug)]
pub enum PlayError {
    /// A framing error occurred.
    Framing(FramingError),
    /// A primitive codec error occurred.
    Codec(CodecError),
    /// The packet ID does not match the expected value.
    WrongPacketId { received: i32, expected: i32 },
    /// The packet contained trailing bytes after the expected fields.
    TrailingBytes { count: usize },
    /// The connection is not in the Play state.
    WrongState { received: ProtocolState },
    /// More input is required.
    Incomplete,
}

impl fmt::Display for PlayError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Framing(error) => write!(formatter, "framing error: {error}"),
            Self::Codec(error) => write!(formatter, "codec error: {error}"),
            Self::WrongPacketId { received, expected } => {
                write!(formatter, "expected packet ID {expected}, got {received}")
            }
            Self::TrailingBytes { count } => {
                write!(formatter, "packet has {count} trailing byte(s)")
            }
            Self::WrongState { received } => {
                write!(formatter, "expected Play state, got {received:?}")
            }
            Self::Incomplete => formatter.write_str("incomplete input"),
        }
    }
}

impl std::error::Error for PlayError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Framing(error) => Some(error),
            Self::Codec(error) => Some(error),
            _ => None,
        }
    }
}

impl From<FramingError> for PlayError {
    fn from(error: FramingError) -> Self {
        match error {
            FramingError::LengthCodec(CodecError::IncompleteInput)
            | FramingError::PacketIdCodec(CodecError::IncompleteInput) => Self::Incomplete,
            other => Self::Framing(other),
        }
    }
}

impl From<CodecError> for PlayError {
    fn from(error: CodecError) -> Self {
        match error {
            CodecError::IncompleteInput => Self::Incomplete,
            other => Self::Codec(other),
        }
    }
}

/// The result of attempting to decode one play packet.
#[derive(Debug, PartialEq)]
pub enum PlayDecodeOutcome {
    /// One complete packet was consumed.
    Complete(PlayPacket),
    /// More bytes are required and no input was consumed.
    Incomplete,
}

/// A decoded play packet.
#[derive(Debug, Clone, PartialEq)]
pub enum PlayPacket {
    /// Clientbound Join Game (Play `0x28`).
    JoinGame(JoinGame),
    /// Clientbound Keep Alive (Play `0x23`).
    KeepAliveClientbound(KeepAlive),
    /// Serverbound Keep Alive (Play `0x12`).
    KeepAliveServerbound(KeepAlive),
    /// Serverbound Set Player Position (Play `0x14`).
    SetPlayerPosition(SetPlayerPosition),
    /// Serverbound Set Player Position and Rotation (Play `0x15`).
    SetPlayerPositionAndRotation(SetPlayerPositionAndRotation),
    /// Serverbound Set Player Rotation (Play `0x16`).
    SetPlayerRotation(SetPlayerRotation),
    /// Clientbound Synchronize Player Position (Play `0x3c`).
    SynchronizePlayerPosition(SynchronizePlayerPosition),
    /// Clientbound Disconnect (Play `0x1a`).
    DisconnectPlay(DisconnectPlay),
    /// Clientbound Chunk Data and Update Light (Play `0x24`).
    ChunkData(ChunkData),
}

/// The player's gamemode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GameMode {
    /// Survival mode.
    Survival,
    /// Creative mode.
    Creative,
    /// Adventure mode.
    Adventure,
    /// Spectator mode.
    Spectator,
}

impl GameMode {
    /// Converts a gamemode to its wire value (u8).
    pub fn to_wire(self) -> u8 {
        match self {
            Self::Survival => 0,
            Self::Creative => 1,
            Self::Adventure => 2,
            Self::Spectator => 3,
        }
    }

    /// Converts a wire value to a gamemode.
    pub fn from_wire(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Survival),
            1 => Some(Self::Creative),
            2 => Some(Self::Adventure),
            3 => Some(Self::Spectator),
            _ => None,
        }
    }
}

/// Clientbound Join Game packet (Play `0x01`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinGame {
    /// The player's entity ID.
    pub entity_id: i32,
    /// Whether the world is hardcore.
    pub is_hardcore: bool,
    /// The player's gamemode.
    pub gamemode: GameMode,
    /// The previous gamemode, or None if there was none.
    pub previous_gamemode: Option<GameMode>,
    /// The list of dimension names (identifiers).
    pub dimension_names: Vec<String>,
    /// The registry codec NBT blob (opaque bytes for now).
    pub registry_codec: Vec<u8>,
    /// The dimension type identifier.
    pub dimension_type: String,
    /// The dimension name identifier.
    pub dimension_name: String,
    /// The hashed seed of the world.
    pub hashed_seed: i64,
    /// The maximum number of players.
    pub max_players: i32,
    /// The view distance (in chunks).
    pub view_distance: i32,
    /// The simulation distance (in chunks).
    pub simulation_distance: i32,
    /// Whether debug info is reduced.
    pub reduce_debug_info: bool,
    /// Whether the respawn screen is enabled.
    pub enable_respawn_screen: bool,
    /// Whether the world is a debug world.
    pub is_debug: bool,
    /// Whether the world is flat.
    pub is_flat: bool,
}

/// Keep Alive packet (clientbound Play `0x23` and serverbound Play `0x12`).
///
/// The clientbound packet carries a payload that the client must echo back
/// unchanged in the serverbound response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeepAlive {
    /// The keep-alive payload (an arbitrary i64 that must be echoed).
    pub payload: i64,
}

/// Serverbound Set Player Position packet (Play `0x14`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SetPlayerPosition {
    /// The player's X coordinate.
    pub x: f64,
    /// The player's Y coordinate.
    pub y: f64,
    /// The player's Z coordinate.
    pub z: f64,
    /// Whether the player is on the ground.
    pub on_ground: bool,
}

/// Serverbound Set Player Position and Rotation packet (Play `0x15`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SetPlayerPositionAndRotation {
    /// The player's X coordinate.
    pub x: f64,
    /// The player's Y coordinate.
    pub y: f64,
    /// The player's Z coordinate.
    pub z: f64,
    /// The player's yaw (rotation around Y axis, in degrees).
    pub yaw: f32,
    /// The player's pitch (rotation around X axis, in degrees).
    pub pitch: f32,
    /// Whether the player is on the ground.
    pub on_ground: bool,
}

/// Serverbound Set Player Rotation packet (Play `0x16`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SetPlayerRotation {
    /// The player's yaw (rotation around Y axis, in degrees).
    pub yaw: f32,
    /// The player's pitch (rotation around X axis, in degrees).
    pub pitch: f32,
    /// Whether the player is on the ground.
    pub on_ground: bool,
}

/// Clientbound Synchronize Player Position packet (Play `0x3c`).
#[derive(Debug, Clone, PartialEq)]
pub struct SynchronizePlayerPosition {
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
    /// Movement flags (bit field: bit 0=X, bit 1=Y, bit 2=Z, bit 3=yaw, bit 4=pitch).
    pub flags: i8,
    /// The teleport ID (must be echoed back in Confirm Teleportation).
    pub teleport_id: i32,
}

/// Clientbound Disconnect (Play) packet (`0x1a`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisconnectPlay {
    /// The disconnect reason (JSON chat component string).
    pub reason: String,
}

/// Clientbound Chunk Data and Update Light packet (Play `0x24`).
///
/// For the initial implementation, the chunk sections and light data are
/// handled as opaque byte arrays. The heightmaps and block entities are
/// also opaque NBT blobs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkData {
    /// The chunk X coordinate.
    pub chunk_x: i32,
    /// The chunk Z coordinate.
    pub chunk_z: i32,
    /// The heightmaps NBT blob (opaque bytes).
    pub heightmaps: Vec<u8>,
    /// The chunk sections and biomes data (opaque bytes).
    pub data: Vec<u8>,
    /// Block entity NBT blobs (opaque bytes, concatenated).
    pub block_entities: Vec<Vec<u8>>,
}

/// Encodes a Join Game packet (clientbound Play `0x01`).
///
/// On error, `output` is unchanged.
pub fn encode_join_game(
    packet: &JoinGame,
    max_frame_length: usize,
    output: &mut Vec<u8>,
) -> Result<(), PlayError> {
    let mut body = Vec::new();
    encode_i32(packet.entity_id, &mut body);
    encode_bool(packet.is_hardcore, &mut body);
    encode_u8(packet.gamemode.to_wire(), &mut body);
    encode_i8(
        packet
            .previous_gamemode
            .map(|g| g.to_wire() as i8)
            .unwrap_or(-1),
        &mut body,
    );

    // Dimension names
    let count = i32::try_from(packet.dimension_names.len())
        .map_err(|_| PlayError::Codec(CodecError::VarIntTooLong))?;
    if packet.dimension_names.len() > MAX_DIMENSION_NAMES {
        return Err(PlayError::Codec(CodecError::StringTooLong));
    }
    encode_var_int(count, &mut body);
    for name in &packet.dimension_names {
        encode_string(name, MAX_IDENTIFIER_LENGTH, &mut body).map_err(PlayError::from)?;
    }

    // Registry codec (opaque bytes)
    encode_byte_array(&packet.registry_codec, MAX_REGISTRY_CODEC_SIZE, &mut body)
        .map_err(PlayError::from)?;

    // Dimension type and name
    encode_string(&packet.dimension_type, MAX_IDENTIFIER_LENGTH, &mut body)
        .map_err(PlayError::from)?;
    encode_string(&packet.dimension_name, MAX_IDENTIFIER_LENGTH, &mut body)
        .map_err(PlayError::from)?;

    // Remaining fields
    encode_i64(packet.hashed_seed, &mut body);
    encode_var_int(packet.max_players, &mut body);
    encode_var_int(packet.view_distance, &mut body);
    encode_var_int(packet.simulation_distance, &mut body);
    encode_bool(packet.reduce_debug_info, &mut body);
    encode_bool(packet.enable_respawn_screen, &mut body);
    encode_bool(packet.is_debug, &mut body);
    encode_bool(packet.is_flat, &mut body);

    encode_frame(JOIN_GAME_PACKET_ID, &body, max_frame_length, output).map_err(PlayError::from)?;
    Ok(())
}

/// Decodes a Join Game packet (clientbound Play `0x01`).
///
/// On [`PlayDecodeOutcome::Incomplete`], the input is unchanged.
pub fn decode_join_game(
    input: &mut &[u8],
    max_frame_length: usize,
) -> Result<PlayDecodeOutcome, PlayError> {
    let source = *input;
    let frame = match decode_frame(input, max_frame_length) {
        Ok(DecodeOutcome::Complete(frame)) => frame,
        Ok(DecodeOutcome::Incomplete) => {
            *input = source;
            return Ok(PlayDecodeOutcome::Incomplete);
        }
        Err(error) => {
            *input = source;
            return Err(PlayError::from(error));
        }
    };

    if frame.packet_id != JOIN_GAME_PACKET_ID {
        *input = source;
        return Err(PlayError::WrongPacketId {
            received: frame.packet_id,
            expected: JOIN_GAME_PACKET_ID,
        });
    }

    let mut body = frame.payload;
    let entity_id = decode_i32(&mut body).map_err(|error| {
        *input = source;
        PlayError::from(error)
    })?;
    let is_hardcore = decode_bool(&mut body).map_err(|error| {
        *input = source;
        PlayError::from(error)
    })?;
    let gamemode_byte = decode_u8(&mut body).map_err(|error| {
        *input = source;
        PlayError::from(error)
    })?;
    let gamemode = GameMode::from_wire(gamemode_byte).ok_or_else(|| {
        *input = source;
        PlayError::Codec(CodecError::InvalidBoolean)
    })?;
    let prev_gamemode_byte = decode_i8(&mut body).map_err(|error| {
        *input = source;
        PlayError::from(error)
    })?;
    let previous_gamemode = if prev_gamemode_byte == -1 {
        None
    } else {
        Some(
            GameMode::from_wire(prev_gamemode_byte as u8).ok_or_else(|| {
                *input = source;
                PlayError::Codec(CodecError::InvalidBoolean)
            })?,
        )
    };

    // Dimension names
    let dim_count = decode_var_int(&mut body).map_err(|error| {
        *input = source;
        PlayError::from(error)
    })?;
    let dim_count = usize::try_from(dim_count).map_err(|_| {
        *input = source;
        PlayError::Codec(CodecError::VarIntTooLong)
    })?;
    if dim_count > MAX_DIMENSION_NAMES {
        *input = source;
        return Err(PlayError::Codec(CodecError::StringTooLong));
    }
    let mut dimension_names = Vec::with_capacity(dim_count);
    for _ in 0..dim_count {
        let name = decode_string(&mut body, MAX_IDENTIFIER_LENGTH).map_err(|error| {
            *input = source;
            PlayError::from(error)
        })?;
        dimension_names.push(name.to_string());
    }

    // Registry codec (opaque bytes)
    let registry_codec =
        decode_byte_array(&mut body, MAX_REGISTRY_CODEC_SIZE).map_err(|error| {
            *input = source;
            PlayError::from(error)
        })?;

    // Dimension type and name
    let dimension_type = decode_string(&mut body, MAX_IDENTIFIER_LENGTH).map_err(|error| {
        *input = source;
        PlayError::from(error)
    })?;
    let dimension_name = decode_string(&mut body, MAX_IDENTIFIER_LENGTH).map_err(|error| {
        *input = source;
        PlayError::from(error)
    })?;

    // Remaining fields
    let hashed_seed = decode_i64(&mut body).map_err(|error| {
        *input = source;
        PlayError::from(error)
    })?;
    let max_players = decode_var_int(&mut body).map_err(|error| {
        *input = source;
        PlayError::from(error)
    })?;
    let view_distance = decode_var_int(&mut body).map_err(|error| {
        *input = source;
        PlayError::from(error)
    })?;
    let simulation_distance = decode_var_int(&mut body).map_err(|error| {
        *input = source;
        PlayError::from(error)
    })?;
    let reduce_debug_info = decode_bool(&mut body).map_err(|error| {
        *input = source;
        PlayError::from(error)
    })?;
    let enable_respawn_screen = decode_bool(&mut body).map_err(|error| {
        *input = source;
        PlayError::from(error)
    })?;
    let is_debug = decode_bool(&mut body).map_err(|error| {
        *input = source;
        PlayError::from(error)
    })?;
    let is_flat = decode_bool(&mut body).map_err(|error| {
        *input = source;
        PlayError::from(error)
    })?;

    if !body.is_empty() {
        *input = source;
        return Err(PlayError::TrailingBytes { count: body.len() });
    }

    Ok(PlayDecodeOutcome::Complete(PlayPacket::JoinGame(
        JoinGame {
            entity_id,
            is_hardcore,
            gamemode,
            previous_gamemode,
            dimension_names,
            registry_codec: registry_codec.to_vec(),
            dimension_type: dimension_type.to_string(),
            dimension_name: dimension_name.to_string(),
            hashed_seed,
            max_players,
            view_distance,
            simulation_distance,
            reduce_debug_info,
            enable_respawn_screen,
            is_debug,
            is_flat,
        },
    )))
}

/// Encodes a clientbound Keep Alive packet (Play `0x1e`).
///
/// On error, `output` is unchanged.
pub fn encode_keep_alive_clientbound(
    packet: &KeepAlive,
    max_frame_length: usize,
    output: &mut Vec<u8>,
) -> Result<(), PlayError> {
    let mut body = Vec::new();
    encode_i64(packet.payload, &mut body);
    encode_frame(
        KEEP_ALIVE_CLIENTBOUND_PACKET_ID,
        &body,
        max_frame_length,
        output,
    )
    .map_err(PlayError::from)?;
    Ok(())
}

/// Decodes a clientbound Keep Alive packet (Play `0x1e`).
///
/// On [`PlayDecodeOutcome::Incomplete`], the input is unchanged.
pub fn decode_keep_alive_clientbound(
    input: &mut &[u8],
    max_frame_length: usize,
) -> Result<PlayDecodeOutcome, PlayError> {
    decode_keep_alive(
        input,
        max_frame_length,
        KEEP_ALIVE_CLIENTBOUND_PACKET_ID,
        true,
    )
}

/// Encodes a serverbound Keep Alive packet (Play `0x14`).
///
/// On error, `output` is unchanged.
pub fn encode_keep_alive_serverbound(
    packet: &KeepAlive,
    max_frame_length: usize,
    output: &mut Vec<u8>,
) -> Result<(), PlayError> {
    let mut body = Vec::new();
    encode_i64(packet.payload, &mut body);
    encode_frame(
        KEEP_ALIVE_SERVERBOUND_PACKET_ID,
        &body,
        max_frame_length,
        output,
    )
    .map_err(PlayError::from)?;
    Ok(())
}

/// Decodes a serverbound Keep Alive packet (Play `0x14`).
///
/// On [`PlayDecodeOutcome::Incomplete`], the input is unchanged.
pub fn decode_keep_alive_serverbound(
    input: &mut &[u8],
    max_frame_length: usize,
) -> Result<PlayDecodeOutcome, PlayError> {
    decode_keep_alive(
        input,
        max_frame_length,
        KEEP_ALIVE_SERVERBOUND_PACKET_ID,
        false,
    )
}

/// Internal helper for decoding Keep Alive packets.
fn decode_keep_alive(
    input: &mut &[u8],
    max_frame_length: usize,
    expected_id: i32,
    clientbound: bool,
) -> Result<PlayDecodeOutcome, PlayError> {
    let source = *input;
    let frame = match decode_frame(input, max_frame_length) {
        Ok(DecodeOutcome::Complete(frame)) => frame,
        Ok(DecodeOutcome::Incomplete) => {
            *input = source;
            return Ok(PlayDecodeOutcome::Incomplete);
        }
        Err(error) => {
            *input = source;
            return Err(PlayError::from(error));
        }
    };

    if frame.packet_id != expected_id {
        *input = source;
        return Err(PlayError::WrongPacketId {
            received: frame.packet_id,
            expected: expected_id,
        });
    }

    let mut body = frame.payload;
    let payload = decode_i64(&mut body).map_err(|error| {
        *input = source;
        PlayError::from(error)
    })?;

    if !body.is_empty() {
        *input = source;
        return Err(PlayError::TrailingBytes { count: body.len() });
    }

    let packet = KeepAlive { payload };
    let wrapped = if clientbound {
        PlayPacket::KeepAliveClientbound(packet)
    } else {
        PlayPacket::KeepAliveServerbound(packet)
    };
    Ok(PlayDecodeOutcome::Complete(wrapped))
}

/// Verifies that the connection is in the Play state.
pub fn ensure_play_state(state: ProtocolState) -> Result<(), PlayError> {
    match state {
        ProtocolState::Play => Ok(()),
        other => Err(PlayError::WrongState { received: other }),
    }
}

// ---------------------------------------------------------------------------
// Set Player Position (serverbound Play 0x14)
// ---------------------------------------------------------------------------

/// Encodes a Set Player Position packet (serverbound Play `0x14`).
pub fn encode_set_player_position(
    packet: &SetPlayerPosition,
    max_frame_length: usize,
    output: &mut Vec<u8>,
) -> Result<(), PlayError> {
    let mut body = Vec::new();
    crate::primitives::encode_f64(packet.x, &mut body);
    crate::primitives::encode_f64(packet.y, &mut body);
    crate::primitives::encode_f64(packet.z, &mut body);
    encode_bool(packet.on_ground, &mut body);
    encode_frame(
        SET_PLAYER_POSITION_PACKET_ID,
        &body,
        max_frame_length,
        output,
    )
    .map_err(PlayError::from)?;
    Ok(())
}

/// Decodes a Set Player Position packet (serverbound Play `0x14`).
pub fn decode_set_player_position(
    input: &mut &[u8],
    max_frame_length: usize,
) -> Result<PlayDecodeOutcome, PlayError> {
    let source = *input;
    let frame = match decode_frame(input, max_frame_length) {
        Ok(DecodeOutcome::Complete(frame)) => frame,
        Ok(DecodeOutcome::Incomplete) => {
            *input = source;
            return Ok(PlayDecodeOutcome::Incomplete);
        }
        Err(error) => {
            *input = source;
            return Err(PlayError::from(error));
        }
    };

    if frame.packet_id != SET_PLAYER_POSITION_PACKET_ID {
        *input = source;
        return Err(PlayError::WrongPacketId {
            received: frame.packet_id,
            expected: SET_PLAYER_POSITION_PACKET_ID,
        });
    }

    let mut body = frame.payload;
    let x = crate::primitives::decode_f64(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let y = crate::primitives::decode_f64(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let z = crate::primitives::decode_f64(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let on_ground = decode_bool(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;

    if !body.is_empty() {
        *input = source;
        return Err(PlayError::TrailingBytes { count: body.len() });
    }

    Ok(PlayDecodeOutcome::Complete(PlayPacket::SetPlayerPosition(
        SetPlayerPosition { x, y, z, on_ground },
    )))
}

// ---------------------------------------------------------------------------
// Set Player Position and Rotation (serverbound Play 0x15)
// ---------------------------------------------------------------------------

/// Encodes a Set Player Position and Rotation packet (serverbound Play `0x15`).
pub fn encode_set_player_position_and_rotation(
    packet: &SetPlayerPositionAndRotation,
    max_frame_length: usize,
    output: &mut Vec<u8>,
) -> Result<(), PlayError> {
    let mut body = Vec::new();
    crate::primitives::encode_f64(packet.x, &mut body);
    crate::primitives::encode_f64(packet.y, &mut body);
    crate::primitives::encode_f64(packet.z, &mut body);
    crate::primitives::encode_f32(packet.yaw, &mut body);
    crate::primitives::encode_f32(packet.pitch, &mut body);
    encode_bool(packet.on_ground, &mut body);
    encode_frame(
        SET_PLAYER_POSITION_AND_ROTATION_PACKET_ID,
        &body,
        max_frame_length,
        output,
    )
    .map_err(PlayError::from)?;
    Ok(())
}

/// Decodes a Set Player Position and Rotation packet (serverbound Play `0x15`).
pub fn decode_set_player_position_and_rotation(
    input: &mut &[u8],
    max_frame_length: usize,
) -> Result<PlayDecodeOutcome, PlayError> {
    let source = *input;
    let frame = match decode_frame(input, max_frame_length) {
        Ok(DecodeOutcome::Complete(frame)) => frame,
        Ok(DecodeOutcome::Incomplete) => {
            *input = source;
            return Ok(PlayDecodeOutcome::Incomplete);
        }
        Err(error) => {
            *input = source;
            return Err(PlayError::from(error));
        }
    };

    if frame.packet_id != SET_PLAYER_POSITION_AND_ROTATION_PACKET_ID {
        *input = source;
        return Err(PlayError::WrongPacketId {
            received: frame.packet_id,
            expected: SET_PLAYER_POSITION_AND_ROTATION_PACKET_ID,
        });
    }

    let mut body = frame.payload;
    let x = crate::primitives::decode_f64(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let y = crate::primitives::decode_f64(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let z = crate::primitives::decode_f64(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let yaw = crate::primitives::decode_f32(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let pitch = crate::primitives::decode_f32(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let on_ground = decode_bool(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;

    if !body.is_empty() {
        *input = source;
        return Err(PlayError::TrailingBytes { count: body.len() });
    }

    Ok(PlayDecodeOutcome::Complete(
        PlayPacket::SetPlayerPositionAndRotation(SetPlayerPositionAndRotation {
            x,
            y,
            z,
            yaw,
            pitch,
            on_ground,
        }),
    ))
}

// ---------------------------------------------------------------------------
// Set Player Rotation (serverbound Play 0x16)
// ---------------------------------------------------------------------------

/// Encodes a Set Player Rotation packet (serverbound Play `0x16`).
pub fn encode_set_player_rotation(
    packet: &SetPlayerRotation,
    max_frame_length: usize,
    output: &mut Vec<u8>,
) -> Result<(), PlayError> {
    let mut body = Vec::new();
    crate::primitives::encode_f32(packet.yaw, &mut body);
    crate::primitives::encode_f32(packet.pitch, &mut body);
    encode_bool(packet.on_ground, &mut body);
    encode_frame(
        SET_PLAYER_ROTATION_PACKET_ID,
        &body,
        max_frame_length,
        output,
    )
    .map_err(PlayError::from)?;
    Ok(())
}

/// Decodes a Set Player Rotation packet (serverbound Play `0x16`).
pub fn decode_set_player_rotation(
    input: &mut &[u8],
    max_frame_length: usize,
) -> Result<PlayDecodeOutcome, PlayError> {
    let source = *input;
    let frame = match decode_frame(input, max_frame_length) {
        Ok(DecodeOutcome::Complete(frame)) => frame,
        Ok(DecodeOutcome::Incomplete) => {
            *input = source;
            return Ok(PlayDecodeOutcome::Incomplete);
        }
        Err(error) => {
            *input = source;
            return Err(PlayError::from(error));
        }
    };

    if frame.packet_id != SET_PLAYER_ROTATION_PACKET_ID {
        *input = source;
        return Err(PlayError::WrongPacketId {
            received: frame.packet_id,
            expected: SET_PLAYER_ROTATION_PACKET_ID,
        });
    }

    let mut body = frame.payload;
    let yaw = crate::primitives::decode_f32(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let pitch = crate::primitives::decode_f32(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let on_ground = decode_bool(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;

    if !body.is_empty() {
        *input = source;
        return Err(PlayError::TrailingBytes { count: body.len() });
    }

    Ok(PlayDecodeOutcome::Complete(PlayPacket::SetPlayerRotation(
        SetPlayerRotation {
            yaw,
            pitch,
            on_ground,
        },
    )))
}

// ---------------------------------------------------------------------------
// Synchronize Player Position (clientbound Play 0x3c)
// ---------------------------------------------------------------------------

/// Encodes a Synchronize Player Position packet (clientbound Play `0x3c`).
pub fn encode_synchronize_player_position(
    packet: &SynchronizePlayerPosition,
    max_frame_length: usize,
    output: &mut Vec<u8>,
) -> Result<(), PlayError> {
    let mut body = Vec::new();
    crate::primitives::encode_f64(packet.x, &mut body);
    crate::primitives::encode_f64(packet.y, &mut body);
    crate::primitives::encode_f64(packet.z, &mut body);
    crate::primitives::encode_f32(packet.yaw, &mut body);
    crate::primitives::encode_f32(packet.pitch, &mut body);
    encode_i8(packet.flags, &mut body);
    encode_var_int(packet.teleport_id, &mut body);
    encode_frame(
        SYNCHRONIZE_PLAYER_POSITION_PACKET_ID,
        &body,
        max_frame_length,
        output,
    )
    .map_err(PlayError::from)?;
    Ok(())
}

/// Decodes a Synchronize Player Position packet (clientbound Play `0x3c`).
pub fn decode_synchronize_player_position(
    input: &mut &[u8],
    max_frame_length: usize,
) -> Result<PlayDecodeOutcome, PlayError> {
    let source = *input;
    let frame = match decode_frame(input, max_frame_length) {
        Ok(DecodeOutcome::Complete(frame)) => frame,
        Ok(DecodeOutcome::Incomplete) => {
            *input = source;
            return Ok(PlayDecodeOutcome::Incomplete);
        }
        Err(error) => {
            *input = source;
            return Err(PlayError::from(error));
        }
    };

    if frame.packet_id != SYNCHRONIZE_PLAYER_POSITION_PACKET_ID {
        *input = source;
        return Err(PlayError::WrongPacketId {
            received: frame.packet_id,
            expected: SYNCHRONIZE_PLAYER_POSITION_PACKET_ID,
        });
    }

    let mut body = frame.payload;
    let x = crate::primitives::decode_f64(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let y = crate::primitives::decode_f64(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let z = crate::primitives::decode_f64(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let yaw = crate::primitives::decode_f32(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let pitch = crate::primitives::decode_f32(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let flags = decode_i8(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let teleport_id = decode_var_int(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;

    if !body.is_empty() {
        *input = source;
        return Err(PlayError::TrailingBytes { count: body.len() });
    }

    Ok(PlayDecodeOutcome::Complete(
        PlayPacket::SynchronizePlayerPosition(SynchronizePlayerPosition {
            x,
            y,
            z,
            yaw,
            pitch,
            flags,
            teleport_id,
        }),
    ))
}

// ---------------------------------------------------------------------------
// Disconnect (Play) (clientbound Play 0x1a)
// ---------------------------------------------------------------------------

/// Encodes a Disconnect (Play) packet (clientbound Play `0x1a`).
pub fn encode_disconnect_play(
    packet: &DisconnectPlay,
    max_frame_length: usize,
    output: &mut Vec<u8>,
) -> Result<(), PlayError> {
    let mut body = Vec::new();
    encode_string(&packet.reason, MAX_CHAT_COMPONENT_LENGTH, &mut body).map_err(PlayError::from)?;
    encode_frame(DISCONNECT_PLAY_PACKET_ID, &body, max_frame_length, output)
        .map_err(PlayError::from)?;
    Ok(())
}

/// Decodes a Disconnect (Play) packet (clientbound Play `0x1a`).
pub fn decode_disconnect_play(
    input: &mut &[u8],
    max_frame_length: usize,
) -> Result<PlayDecodeOutcome, PlayError> {
    let source = *input;
    let frame = match decode_frame(input, max_frame_length) {
        Ok(DecodeOutcome::Complete(frame)) => frame,
        Ok(DecodeOutcome::Incomplete) => {
            *input = source;
            return Ok(PlayDecodeOutcome::Incomplete);
        }
        Err(error) => {
            *input = source;
            return Err(PlayError::from(error));
        }
    };

    if frame.packet_id != DISCONNECT_PLAY_PACKET_ID {
        *input = source;
        return Err(PlayError::WrongPacketId {
            received: frame.packet_id,
            expected: DISCONNECT_PLAY_PACKET_ID,
        });
    }

    let mut body = frame.payload;
    let reason = decode_string(&mut body, MAX_CHAT_COMPONENT_LENGTH).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;

    if !body.is_empty() {
        *input = source;
        return Err(PlayError::TrailingBytes { count: body.len() });
    }

    Ok(PlayDecodeOutcome::Complete(PlayPacket::DisconnectPlay(
        DisconnectPlay {
            reason: reason.to_string(),
        },
    )))
}

// ---------------------------------------------------------------------------
// Chunk Data and Update Light (clientbound Play 0x24)
// ---------------------------------------------------------------------------

/// Encodes a Chunk Data and Update Light packet (clientbound Play `0x24`).
///
/// For the initial implementation, chunk sections, heightmaps, and block
/// entities are encoded as opaque byte arrays.
pub fn encode_chunk_data(
    packet: &ChunkData,
    max_frame_length: usize,
    output: &mut Vec<u8>,
) -> Result<(), PlayError> {
    let mut body = Vec::new();
    encode_i32(packet.chunk_x, &mut body);
    encode_i32(packet.chunk_z, &mut body);

    // Heightmaps (NBT blob)
    encode_byte_array(&packet.heightmaps, MAX_CHUNK_DATA_SIZE, &mut body)
        .map_err(PlayError::from)?;

    // Chunk sections and biomes (opaque)
    encode_byte_array(&packet.data, MAX_CHUNK_DATA_SIZE, &mut body).map_err(PlayError::from)?;

    // Block entities
    let count = i32::try_from(packet.block_entities.len())
        .map_err(|_| PlayError::Codec(CodecError::VarIntTooLong))?;
    if packet.block_entities.len() > MAX_BLOCK_ENTITIES {
        return Err(PlayError::Codec(CodecError::StringTooLong));
    }
    encode_var_int(count, &mut body);
    for be in &packet.block_entities {
        encode_byte_array(be, MAX_CHUNK_DATA_SIZE, &mut body).map_err(PlayError::from)?;
    }

    // Light data (opaque, empty for now)
    encode_byte_array(&[], MAX_CHUNK_DATA_SIZE, &mut body).map_err(PlayError::from)?;

    encode_frame(CHUNK_DATA_PACKET_ID, &body, max_frame_length, output).map_err(PlayError::from)?;
    Ok(())
}

/// Decodes a Chunk Data and Update Light packet (clientbound Play `0x24`).
pub fn decode_chunk_data(
    input: &mut &[u8],
    max_frame_length: usize,
) -> Result<PlayDecodeOutcome, PlayError> {
    let source = *input;
    let frame = match decode_frame(input, max_frame_length) {
        Ok(DecodeOutcome::Complete(frame)) => frame,
        Ok(DecodeOutcome::Incomplete) => {
            *input = source;
            return Ok(PlayDecodeOutcome::Incomplete);
        }
        Err(error) => {
            *input = source;
            return Err(PlayError::from(error));
        }
    };

    if frame.packet_id != CHUNK_DATA_PACKET_ID {
        *input = source;
        return Err(PlayError::WrongPacketId {
            received: frame.packet_id,
            expected: CHUNK_DATA_PACKET_ID,
        });
    }

    let mut body = frame.payload;
    let chunk_x = decode_i32(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let chunk_z = decode_i32(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let heightmaps = decode_byte_array(&mut body, MAX_CHUNK_DATA_SIZE).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let data = decode_byte_array(&mut body, MAX_CHUNK_DATA_SIZE).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;

    let be_count = decode_var_int(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let be_count = usize::try_from(be_count).map_err(|_| {
        *input = source;
        PlayError::Codec(CodecError::VarIntTooLong)
    })?;
    if be_count > MAX_BLOCK_ENTITIES {
        *input = source;
        return Err(PlayError::Codec(CodecError::StringTooLong));
    }
    let mut block_entities = Vec::with_capacity(be_count);
    for _ in 0..be_count {
        let be = decode_byte_array(&mut body, MAX_CHUNK_DATA_SIZE).map_err(|e| {
            *input = source;
            PlayError::from(e)
        })?;
        block_entities.push(be.to_vec());
    }

    // Light data (opaque, skip for now)
    let _light = decode_byte_array(&mut body, MAX_CHUNK_DATA_SIZE).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;

    if !body.is_empty() {
        *input = source;
        return Err(PlayError::TrailingBytes { count: body.len() });
    }

    Ok(PlayDecodeOutcome::Complete(PlayPacket::ChunkData(
        ChunkData {
            chunk_x,
            chunk_z,
            heightmaps: heightmaps.to_vec(),
            data: data.to_vec(),
            block_entities,
        },
    )))
}

#[cfg(test)]
mod tests {
    use super::{
        ChunkData, DisconnectPlay, GameMode, JOIN_GAME_PACKET_ID, JoinGame,
        KEEP_ALIVE_CLIENTBOUND_PACKET_ID, KeepAlive, PlayDecodeOutcome, PlayError, PlayPacket,
        SetPlayerPosition, SetPlayerPositionAndRotation, SetPlayerRotation,
        SynchronizePlayerPosition, decode_chunk_data, decode_disconnect_play, decode_join_game,
        decode_keep_alive_clientbound, decode_keep_alive_serverbound, decode_set_player_position,
        decode_set_player_position_and_rotation, decode_set_player_rotation,
        decode_synchronize_player_position, encode_chunk_data, encode_disconnect_play,
        encode_join_game, encode_keep_alive_clientbound, encode_keep_alive_serverbound,
        encode_set_player_position, encode_set_player_position_and_rotation,
        encode_set_player_rotation, encode_synchronize_player_position, ensure_play_state,
    };
    use crate::state::ProtocolState;

    const TEST_MAX_FRAME: usize = 1048576;

    fn sample_join_game() -> JoinGame {
        JoinGame {
            entity_id: 42,
            is_hardcore: false,
            gamemode: GameMode::Survival,
            previous_gamemode: None,
            dimension_names: vec!["minecraft:overworld".to_string()],
            registry_codec: Vec::new(),
            dimension_type: "minecraft:overworld".to_string(),
            dimension_name: "minecraft:overworld".to_string(),
            hashed_seed: 0,
            max_players: 20,
            view_distance: 10,
            simulation_distance: 10,
            reduce_debug_info: false,
            enable_respawn_screen: true,
            is_debug: false,
            is_flat: false,
        }
    }

    #[test]
    fn join_game_round_trips() -> Result<(), PlayError> {
        let packet = sample_join_game();
        let mut wire = Vec::new();
        encode_join_game(&packet, TEST_MAX_FRAME, &mut wire)?;

        let mut input = wire.as_slice();
        match decode_join_game(&mut input, TEST_MAX_FRAME)? {
            PlayDecodeOutcome::Complete(PlayPacket::JoinGame(decoded)) => {
                assert_eq!(decoded, packet);
            }
            _ => panic!("expected JoinGame"),
        }
        assert!(input.is_empty());
        Ok(())
    }

    #[test]
    fn join_game_with_multiple_dimensions_round_trips() -> Result<(), PlayError> {
        let mut packet = sample_join_game();
        packet.dimension_names = vec![
            "minecraft:overworld".to_string(),
            "minecraft:the_nether".to_string(),
            "minecraft:the_end".to_string(),
        ];
        packet.gamemode = GameMode::Creative;
        packet.previous_gamemode = Some(GameMode::Survival);
        packet.is_hardcore = true;
        packet.hashed_seed = 123456789;
        packet.view_distance = 32;
        packet.simulation_distance = 8;
        packet.reduce_debug_info = true;
        packet.is_flat = true;

        let mut wire = Vec::new();
        encode_join_game(&packet, TEST_MAX_FRAME, &mut wire)?;

        let mut input = wire.as_slice();
        match decode_join_game(&mut input, TEST_MAX_FRAME)? {
            PlayDecodeOutcome::Complete(PlayPacket::JoinGame(decoded)) => {
                assert_eq!(decoded, packet);
            }
            _ => panic!("expected JoinGame"),
        }
        assert!(input.is_empty());
        Ok(())
    }

    #[test]
    fn join_game_with_registry_codec_round_trips() -> Result<(), PlayError> {
        let mut packet = sample_join_game();
        packet.registry_codec = vec![0x0a, 0x00, 0x00, 0x00]; // minimal NBT stub

        let mut wire = Vec::new();
        encode_join_game(&packet, TEST_MAX_FRAME, &mut wire)?;

        let mut input = wire.as_slice();
        match decode_join_game(&mut input, TEST_MAX_FRAME)? {
            PlayDecodeOutcome::Complete(PlayPacket::JoinGame(decoded)) => {
                assert_eq!(decoded, packet);
            }
            _ => panic!("expected JoinGame"),
        }
        Ok(())
    }

    #[test]
    fn join_game_wrong_packet_id_is_rejected() -> Result<(), PlayError> {
        let mut body = Vec::new();
        crate::primitives::encode_i32(42, &mut body);
        crate::primitives::encode_bool(false, &mut body);
        crate::primitives::encode_u8(0, &mut body);
        crate::primitives::encode_i8(-1, &mut body);
        crate::primitives::encode_var_int(0, &mut body);
        crate::primitives::encode_byte_array(&[], 1048576, &mut body).map_err(PlayError::from)?;
        crate::primitives::encode_string("minecraft:overworld", 32767, &mut body)
            .map_err(PlayError::from)?;
        crate::primitives::encode_string("minecraft:overworld", 32767, &mut body)
            .map_err(PlayError::from)?;
        crate::primitives::encode_i64(0, &mut body);
        crate::primitives::encode_var_int(20, &mut body);
        crate::primitives::encode_var_int(10, &mut body);
        crate::primitives::encode_var_int(10, &mut body);
        crate::primitives::encode_bool(false, &mut body);
        crate::primitives::encode_bool(true, &mut body);
        crate::primitives::encode_bool(false, &mut body);
        crate::primitives::encode_bool(false, &mut body);

        let mut wire = Vec::new();
        crate::framing::encode_frame(0x05, &body, TEST_MAX_FRAME, &mut wire)
            .map_err(PlayError::from)?;

        let mut input = wire.as_slice();
        let result = decode_join_game(&mut input, TEST_MAX_FRAME);
        assert!(matches!(result, Err(PlayError::WrongPacketId { .. })));
        assert_eq!(input, wire.as_slice());
        Ok(())
    }

    #[test]
    fn join_game_truncated_is_incomplete() -> Result<(), PlayError> {
        let packet = sample_join_game();
        let mut wire = Vec::new();
        encode_join_game(&packet, TEST_MAX_FRAME, &mut wire)?;

        for split in 0..wire.len() {
            let mut input = &wire[..split];
            assert_eq!(
                decode_join_game(&mut input, TEST_MAX_FRAME)?,
                PlayDecodeOutcome::Incomplete
            );
            assert_eq!(input, &wire[..split]);
        }
        Ok(())
    }

    #[test]
    fn join_game_trailing_bytes_are_rejected() -> Result<(), PlayError> {
        let packet = sample_join_game();
        let mut body = Vec::new();
        crate::primitives::encode_i32(packet.entity_id, &mut body);
        crate::primitives::encode_bool(packet.is_hardcore, &mut body);
        crate::primitives::encode_u8(packet.gamemode.to_wire(), &mut body);
        crate::primitives::encode_i8(-1, &mut body);
        crate::primitives::encode_var_int(1, &mut body);
        crate::primitives::encode_string("minecraft:overworld", 32767, &mut body)
            .map_err(PlayError::from)?;
        crate::primitives::encode_byte_array(&[], 1048576, &mut body).map_err(PlayError::from)?;
        crate::primitives::encode_string("minecraft:overworld", 32767, &mut body)
            .map_err(PlayError::from)?;
        crate::primitives::encode_string("minecraft:overworld", 32767, &mut body)
            .map_err(PlayError::from)?;
        crate::primitives::encode_i64(0, &mut body);
        crate::primitives::encode_var_int(20, &mut body);
        crate::primitives::encode_var_int(10, &mut body);
        crate::primitives::encode_var_int(10, &mut body);
        crate::primitives::encode_bool(false, &mut body);
        crate::primitives::encode_bool(true, &mut body);
        crate::primitives::encode_bool(false, &mut body);
        crate::primitives::encode_bool(false, &mut body);
        body.push(0xff); // trailing byte

        let mut wire = Vec::new();
        crate::framing::encode_frame(JOIN_GAME_PACKET_ID, &body, TEST_MAX_FRAME, &mut wire)
            .map_err(PlayError::from)?;

        let mut input = wire.as_slice();
        let result = decode_join_game(&mut input, TEST_MAX_FRAME);
        assert!(matches!(result, Err(PlayError::TrailingBytes { .. })));
        Ok(())
    }

    #[test]
    fn game_mode_wire_values_are_stable() {
        assert_eq!(GameMode::Survival.to_wire(), 0);
        assert_eq!(GameMode::Creative.to_wire(), 1);
        assert_eq!(GameMode::Adventure.to_wire(), 2);
        assert_eq!(GameMode::Spectator.to_wire(), 3);
        assert_eq!(GameMode::from_wire(0), Some(GameMode::Survival));
        assert_eq!(GameMode::from_wire(4), None);
    }

    #[test]
    fn ensure_play_state_accepts_play() {
        assert!(ensure_play_state(ProtocolState::Play).is_ok());
    }

    #[test]
    fn ensure_play_state_rejects_other_states() {
        for state in [
            ProtocolState::Handshaking,
            ProtocolState::Status,
            ProtocolState::Login,
            ProtocolState::Closed,
        ] {
            assert!(ensure_play_state(state).is_err());
        }
    }

    #[test]
    fn keep_alive_clientbound_round_trips() -> Result<(), PlayError> {
        let packet = KeepAlive { payload: 12345 };
        let mut wire = Vec::new();
        encode_keep_alive_clientbound(&packet, TEST_MAX_FRAME, &mut wire)?;

        let mut input = wire.as_slice();
        match decode_keep_alive_clientbound(&mut input, TEST_MAX_FRAME)? {
            PlayDecodeOutcome::Complete(PlayPacket::KeepAliveClientbound(decoded)) => {
                assert_eq!(decoded, packet);
            }
            _ => panic!("expected KeepAliveClientbound"),
        }
        assert!(input.is_empty());
        Ok(())
    }

    #[test]
    fn keep_alive_serverbound_round_trips() -> Result<(), PlayError> {
        let packet = KeepAlive { payload: -1 };
        let mut wire = Vec::new();
        encode_keep_alive_serverbound(&packet, TEST_MAX_FRAME, &mut wire)?;

        let mut input = wire.as_slice();
        match decode_keep_alive_serverbound(&mut input, TEST_MAX_FRAME)? {
            PlayDecodeOutcome::Complete(PlayPacket::KeepAliveServerbound(decoded)) => {
                assert_eq!(decoded, packet);
            }
            _ => panic!("expected KeepAliveServerbound"),
        }
        assert!(input.is_empty());
        Ok(())
    }

    #[test]
    fn keep_alive_boundary_payloads_round_trip() -> Result<(), PlayError> {
        for payload in [0i64, -1, i64::MAX, i64::MIN] {
            let packet = KeepAlive { payload };
            let mut wire = Vec::new();
            encode_keep_alive_clientbound(&packet, TEST_MAX_FRAME, &mut wire)?;

            let mut input = wire.as_slice();
            match decode_keep_alive_clientbound(&mut input, TEST_MAX_FRAME)? {
                PlayDecodeOutcome::Complete(PlayPacket::KeepAliveClientbound(decoded)) => {
                    assert_eq!(decoded.payload, payload);
                }
                _ => panic!("expected KeepAliveClientbound"),
            }
        }
        Ok(())
    }

    #[test]
    fn keep_alive_clientbound_wrong_packet_id_is_rejected() -> Result<(), PlayError> {
        let mut body = Vec::new();
        crate::primitives::encode_i64(42, &mut body);

        let mut wire = Vec::new();
        crate::framing::encode_frame(0x05, &body, TEST_MAX_FRAME, &mut wire)
            .map_err(PlayError::from)?;

        let mut input = wire.as_slice();
        let result = decode_keep_alive_clientbound(&mut input, TEST_MAX_FRAME);
        assert!(matches!(result, Err(PlayError::WrongPacketId { .. })));
        Ok(())
    }

    #[test]
    fn keep_alive_truncated_is_incomplete() -> Result<(), PlayError> {
        let packet = KeepAlive { payload: 42 };
        let mut wire = Vec::new();
        encode_keep_alive_clientbound(&packet, TEST_MAX_FRAME, &mut wire)?;

        for split in 0..wire.len() {
            let mut input = &wire[..split];
            assert_eq!(
                decode_keep_alive_clientbound(&mut input, TEST_MAX_FRAME)?,
                PlayDecodeOutcome::Incomplete
            );
            assert_eq!(input, &wire[..split]);
        }
        Ok(())
    }

    #[test]
    fn keep_alive_trailing_bytes_are_rejected() -> Result<(), PlayError> {
        let mut body = Vec::new();
        crate::primitives::encode_i64(42, &mut body);
        body.push(0xff); // trailing byte

        let mut wire = Vec::new();
        crate::framing::encode_frame(
            KEEP_ALIVE_CLIENTBOUND_PACKET_ID,
            &body,
            TEST_MAX_FRAME,
            &mut wire,
        )
        .map_err(PlayError::from)?;

        let mut input = wire.as_slice();
        let result = decode_keep_alive_clientbound(&mut input, TEST_MAX_FRAME);
        assert!(matches!(result, Err(PlayError::TrailingBytes { .. })));
        Ok(())
    }

    #[test]
    fn keep_alive_serverbound_wrong_packet_id_is_rejected() -> Result<(), PlayError> {
        let mut body = Vec::new();
        crate::primitives::encode_i64(42, &mut body);

        let mut wire = Vec::new();
        crate::framing::encode_frame(0x05, &body, TEST_MAX_FRAME, &mut wire)
            .map_err(PlayError::from)?;

        let mut input = wire.as_slice();
        let result = decode_keep_alive_serverbound(&mut input, TEST_MAX_FRAME);
        assert!(matches!(result, Err(PlayError::WrongPacketId { .. })));
        Ok(())
    }

    #[test]
    fn set_player_position_round_trips() -> Result<(), PlayError> {
        let packet = SetPlayerPosition {
            x: 1.5,
            y: 64.0,
            z: -2.25,
            on_ground: true,
        };
        let mut wire = Vec::new();
        encode_set_player_position(&packet, TEST_MAX_FRAME, &mut wire)?;

        let mut input = wire.as_slice();
        match decode_set_player_position(&mut input, TEST_MAX_FRAME)? {
            PlayDecodeOutcome::Complete(PlayPacket::SetPlayerPosition(decoded)) => {
                assert_eq!(decoded, packet);
            }
            _ => panic!("expected SetPlayerPosition"),
        }
        assert!(input.is_empty());
        Ok(())
    }

    #[test]
    fn set_player_position_and_rotation_round_trips() -> Result<(), PlayError> {
        let packet = SetPlayerPositionAndRotation {
            x: 100.5,
            y: -64.0,
            z: 0.0,
            yaw: 90.0,
            pitch: -45.0,
            on_ground: false,
        };
        let mut wire = Vec::new();
        encode_set_player_position_and_rotation(&packet, TEST_MAX_FRAME, &mut wire)?;

        let mut input = wire.as_slice();
        match decode_set_player_position_and_rotation(&mut input, TEST_MAX_FRAME)? {
            PlayDecodeOutcome::Complete(PlayPacket::SetPlayerPositionAndRotation(decoded)) => {
                assert_eq!(decoded, packet);
            }
            _ => panic!("expected SetPlayerPositionAndRotation"),
        }
        assert!(input.is_empty());
        Ok(())
    }

    #[test]
    fn set_player_rotation_round_trips() -> Result<(), PlayError> {
        let packet = SetPlayerRotation {
            yaw: 180.0,
            pitch: 0.0,
            on_ground: true,
        };
        let mut wire = Vec::new();
        encode_set_player_rotation(&packet, TEST_MAX_FRAME, &mut wire)?;

        let mut input = wire.as_slice();
        match decode_set_player_rotation(&mut input, TEST_MAX_FRAME)? {
            PlayDecodeOutcome::Complete(PlayPacket::SetPlayerRotation(decoded)) => {
                assert_eq!(decoded, packet);
            }
            _ => panic!("expected SetPlayerRotation"),
        }
        assert!(input.is_empty());
        Ok(())
    }

    #[test]
    fn synchronize_player_position_round_trips() -> Result<(), PlayError> {
        let packet = SynchronizePlayerPosition {
            x: 0.0,
            y: 70.0,
            z: 0.0,
            yaw: 0.0,
            pitch: 0.0,
            flags: 0x00,
            teleport_id: 42,
        };
        let mut wire = Vec::new();
        encode_synchronize_player_position(&packet, TEST_MAX_FRAME, &mut wire)?;

        let mut input = wire.as_slice();
        match decode_synchronize_player_position(&mut input, TEST_MAX_FRAME)? {
            PlayDecodeOutcome::Complete(PlayPacket::SynchronizePlayerPosition(decoded)) => {
                assert_eq!(decoded, packet);
            }
            _ => panic!("expected SynchronizePlayerPosition"),
        }
        assert!(input.is_empty());
        Ok(())
    }

    #[test]
    fn set_player_position_truncated_is_incomplete() -> Result<(), PlayError> {
        let packet = SetPlayerPosition {
            x: 1.0,
            y: 2.0,
            z: 3.0,
            on_ground: true,
        };
        let mut wire = Vec::new();
        encode_set_player_position(&packet, TEST_MAX_FRAME, &mut wire)?;

        for split in 0..wire.len() {
            let mut input = &wire[..split];
            assert_eq!(
                decode_set_player_position(&mut input, TEST_MAX_FRAME)?,
                PlayDecodeOutcome::Incomplete
            );
            assert_eq!(input, &wire[..split]);
        }
        Ok(())
    }

    #[test]
    fn set_player_position_wrong_packet_id_is_rejected() -> Result<(), PlayError> {
        let mut body = Vec::new();
        crate::primitives::encode_f64(1.0, &mut body);
        crate::primitives::encode_f64(2.0, &mut body);
        crate::primitives::encode_f64(3.0, &mut body);
        crate::primitives::encode_bool(true, &mut body);

        let mut wire = Vec::new();
        crate::framing::encode_frame(0x05, &body, TEST_MAX_FRAME, &mut wire)
            .map_err(PlayError::from)?;

        let mut input = wire.as_slice();
        let result = decode_set_player_position(&mut input, TEST_MAX_FRAME);
        assert!(matches!(result, Err(PlayError::WrongPacketId { .. })));
        Ok(())
    }

    #[test]
    fn synchronize_player_position_with_flags_round_trips() -> Result<(), PlayError> {
        let packet = SynchronizePlayerPosition {
            x: -100.5,
            y: 64.0,
            z: 200.25,
            yaw: 45.5,
            pitch: -30.0,
            flags: 0x1f, // all relative
            teleport_id: 7,
        };
        let mut wire = Vec::new();
        encode_synchronize_player_position(&packet, TEST_MAX_FRAME, &mut wire)?;

        let mut input = wire.as_slice();
        match decode_synchronize_player_position(&mut input, TEST_MAX_FRAME)? {
            PlayDecodeOutcome::Complete(PlayPacket::SynchronizePlayerPosition(decoded)) => {
                assert_eq!(decoded, packet);
            }
            _ => panic!("expected SynchronizePlayerPosition"),
        }
        Ok(())
    }

    #[test]
    fn disconnect_play_round_trips() -> Result<(), PlayError> {
        let packet = DisconnectPlay {
            reason: r#"{"text":"Kicked!"}"#.to_string(),
        };
        let mut wire = Vec::new();
        encode_disconnect_play(&packet, TEST_MAX_FRAME, &mut wire)?;

        let mut input = wire.as_slice();
        match decode_disconnect_play(&mut input, TEST_MAX_FRAME)? {
            PlayDecodeOutcome::Complete(PlayPacket::DisconnectPlay(decoded)) => {
                assert_eq!(decoded, packet);
            }
            _ => panic!("expected DisconnectPlay"),
        }
        assert!(input.is_empty());
        Ok(())
    }

    #[test]
    fn disconnect_play_wrong_packet_id_is_rejected() -> Result<(), PlayError> {
        let mut body = Vec::new();
        crate::primitives::encode_string(r#"{"text":"test"}"#, 32767, &mut body)
            .map_err(PlayError::from)?;

        let mut wire = Vec::new();
        crate::framing::encode_frame(0x05, &body, TEST_MAX_FRAME, &mut wire)
            .map_err(PlayError::from)?;

        let mut input = wire.as_slice();
        let result = decode_disconnect_play(&mut input, TEST_MAX_FRAME);
        assert!(matches!(result, Err(PlayError::WrongPacketId { .. })));
        Ok(())
    }

    #[test]
    fn disconnect_play_truncated_is_incomplete() -> Result<(), PlayError> {
        let packet = DisconnectPlay {
            reason: "test".to_string(),
        };
        let mut wire = Vec::new();
        encode_disconnect_play(&packet, TEST_MAX_FRAME, &mut wire)?;

        for split in 0..wire.len() {
            let mut input = &wire[..split];
            assert_eq!(
                decode_disconnect_play(&mut input, TEST_MAX_FRAME)?,
                PlayDecodeOutcome::Incomplete
            );
        }
        Ok(())
    }

    #[test]
    fn chunk_data_empty_round_trips() -> Result<(), PlayError> {
        let packet = ChunkData {
            chunk_x: 0,
            chunk_z: 0,
            heightmaps: Vec::new(),
            data: Vec::new(),
            block_entities: Vec::new(),
        };
        let mut wire = Vec::new();
        encode_chunk_data(&packet, TEST_MAX_FRAME, &mut wire)?;

        let mut input = wire.as_slice();
        match decode_chunk_data(&mut input, TEST_MAX_FRAME)? {
            PlayDecodeOutcome::Complete(PlayPacket::ChunkData(decoded)) => {
                assert_eq!(decoded, packet);
            }
            _ => panic!("expected ChunkData"),
        }
        assert!(input.is_empty());
        Ok(())
    }

    #[test]
    fn chunk_data_with_block_entities_round_trips() -> Result<(), PlayError> {
        let packet = ChunkData {
            chunk_x: -10,
            chunk_z: 20,
            heightmaps: vec![0x0a, 0x00, 0x00],
            data: vec![0x01, 0x02, 0x03, 0x04],
            block_entities: vec![vec![0x0a, 0x01], vec![0x0a, 0x02]],
        };
        let mut wire = Vec::new();
        encode_chunk_data(&packet, TEST_MAX_FRAME, &mut wire)?;

        let mut input = wire.as_slice();
        match decode_chunk_data(&mut input, TEST_MAX_FRAME)? {
            PlayDecodeOutcome::Complete(PlayPacket::ChunkData(decoded)) => {
                assert_eq!(decoded, packet);
            }
            _ => panic!("expected ChunkData"),
        }
        assert!(input.is_empty());
        Ok(())
    }

    #[test]
    fn chunk_data_wrong_packet_id_is_rejected() -> Result<(), PlayError> {
        let mut body = Vec::new();
        crate::primitives::encode_i32(0, &mut body);
        crate::primitives::encode_i32(0, &mut body);
        crate::primitives::encode_byte_array(&[], 1048576, &mut body).map_err(PlayError::from)?;
        crate::primitives::encode_byte_array(&[], 1048576, &mut body).map_err(PlayError::from)?;
        crate::primitives::encode_var_int(0, &mut body);
        crate::primitives::encode_byte_array(&[], 1048576, &mut body).map_err(PlayError::from)?;

        let mut wire = Vec::new();
        crate::framing::encode_frame(0x05, &body, TEST_MAX_FRAME, &mut wire)
            .map_err(PlayError::from)?;

        let mut input = wire.as_slice();
        let result = decode_chunk_data(&mut input, TEST_MAX_FRAME);
        assert!(matches!(result, Err(PlayError::WrongPacketId { .. })));
        Ok(())
    }

    #[test]
    fn chunk_data_truncated_is_incomplete() -> Result<(), PlayError> {
        let packet = ChunkData {
            chunk_x: 0,
            chunk_z: 0,
            heightmaps: vec![0x0a],
            data: vec![0x01],
            block_entities: Vec::new(),
        };
        let mut wire = Vec::new();
        encode_chunk_data(&packet, TEST_MAX_FRAME, &mut wire)?;

        for split in 0..wire.len() {
            let mut input = &wire[..split];
            assert_eq!(
                decode_chunk_data(&mut input, TEST_MAX_FRAME)?,
                PlayDecodeOutcome::Incomplete
            );
        }
        Ok(())
    }
}
