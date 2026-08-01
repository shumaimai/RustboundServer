//! Play state packet codecs for protocol 763 (Minecraft Java Edition 1.20.1).
//!
//! This module implements the Join Game (clientbound Play `0x01`) packet,
//! which is the first packet sent after Login Success and transitions the
//! client into the Play state.

use std::fmt;

use crate::framing::{DecodeOutcome, FramingError, decode_frame, encode_frame};
use crate::primitives::{
    CodecError, decode_bool, decode_byte_array, decode_i8, decode_i32, decode_i64, decode_string,
    decode_u8, decode_var_int, encode_bool, encode_byte_array, encode_i8, encode_i32, encode_i64,
    encode_string, encode_u8, encode_var_int,
};
use crate::state::ProtocolState;

/// Packet ID for the clientbound Join Game packet.
pub const JOIN_GAME_PACKET_ID: i32 = 0x01;

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
#[derive(Debug, PartialEq, Eq)]
pub enum PlayDecodeOutcome {
    /// One complete packet was consumed.
    Complete(PlayPacket),
    /// More bytes are required and no input was consumed.
    Incomplete,
}

/// A decoded play packet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlayPacket {
    /// Clientbound Join Game (Play `0x01`).
    JoinGame(JoinGame),
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

/// Verifies that the connection is in the Play state.
pub fn ensure_play_state(state: ProtocolState) -> Result<(), PlayError> {
    match state {
        ProtocolState::Play => Ok(()),
        other => Err(PlayError::WrongState { received: other }),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        GameMode, JOIN_GAME_PACKET_ID, JoinGame, PlayDecodeOutcome, PlayError, PlayPacket,
        decode_join_game, encode_join_game, ensure_play_state,
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
}
