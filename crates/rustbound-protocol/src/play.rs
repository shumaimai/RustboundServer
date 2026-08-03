//! Play state packet codecs for protocol 763 (Minecraft Java Edition 1.20.1).
//!
//! This module implements the Join Game (clientbound Play `0x01`) packet,
//! which is the first packet sent after Login Success and transitions the
//! client into the Play state.

use std::fmt;

use crate::framing::{DecodeOutcome, FramingError, decode_frame, encode_frame};
use crate::primitives::{
    CodecError, MAX_CHAT_COMPONENT_LENGTH, Uuid, decode_bool, decode_byte_array, decode_f32,
    decode_f64, decode_i8, decode_i16, decode_i32, decode_i64, decode_position, decode_string,
    decode_u8, decode_var_int, encode_bool, encode_byte_array, encode_f32, encode_f64, encode_i8,
    encode_i16, encode_i32, encode_i64, encode_position, encode_string, encode_u8, encode_var_int,
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

/// Packet ID for the serverbound Confirm Teleportation packet.
pub const CONFIRM_TELEPORTATION_PACKET_ID: i32 = 0x00;

/// Packet ID for the serverbound Client Information packet.
pub const CLIENT_INFORMATION_PACKET_ID: i32 = 0x08;

/// Packet ID for the clientbound Plugin Message (Play) packet.
pub const PLUGIN_MESSAGE_CLIENTBOUND_PACKET_ID: i32 = 0x17;

/// Packet ID for the clientbound Change Difficulty packet.
pub const CHANGE_DIFFICULTY_PACKET_ID: i32 = 0x0C;

/// Packet ID for the clientbound Player Abilities packet.
pub const PLAYER_ABILITIES_PACKET_ID: i32 = 0x34;

/// Packet ID for the clientbound Set Held Item packet.
pub const SET_HELD_ITEM_PACKET_ID: i32 = 0x4D;

/// Packet ID for the clientbound Entity Event (Entity Status) packet.
pub const ENTITY_EVENT_PACKET_ID: i32 = 0x1C;

/// Packet ID for the clientbound Declare Commands packet.
pub const DECLARE_COMMANDS_PACKET_ID: i32 = 0x10;

/// Packet ID for the clientbound Player Info Update packet.
pub const PLAYER_INFO_UPDATE_PACKET_ID: i32 = 0x3A;

/// Packet ID for the clientbound Player Info Remove packet.
pub const PLAYER_INFO_REMOVE_PACKET_ID: i32 = 0x39;

/// Packet ID for the clientbound Spawn Player packet.
pub const SPAWN_PLAYER_PACKET_ID: i32 = 0x03;

/// Packet ID for the clientbound Remove Entities packet.
pub const REMOVE_ENTITIES_PACKET_ID: i32 = 0x3E;

/// Packet ID for the clientbound Set Default Spawn Position packet.
pub const SET_DEFAULT_SPAWN_POSITION_PACKET_ID: i32 = 0x50;

/// Packet ID for the clientbound Game Event packet.
pub const GAME_EVENT_PACKET_ID: i32 = 0x1F;

/// Packet ID for the clientbound Set Center Chunk packet.
pub const SET_CENTER_CHUNK_PACKET_ID: i32 = 0x4E;

/// Packet ID for the clientbound Unload Chunk packet.
pub const UNLOAD_CHUNK_PACKET_ID: i32 = 0x1E;

/// Packet ID for the clientbound Combat Death packet.
pub const COMBAT_DEATH_PACKET_ID: i32 = 0x38;

/// Packet ID for the clientbound Set Block Destroy Stage packet.
pub const SET_BLOCK_DESTROY_STAGE_PACKET_ID: i32 = 0x07;

/// Packet ID for the clientbound Set Render Distance packet.
pub const SET_RENDER_DISTANCE_PACKET_ID: i32 = 0x4F;

/// Packet ID for the clientbound Set Simulation Distance packet.
pub const SET_SIMULATION_DISTANCE_PACKET_ID: i32 = 0x5C;

/// Packet ID for the clientbound Update Tags packet (protocol 763).
pub const UPDATE_TAGS_PACKET_ID: i32 = 0x68;

/// Packet ID for the clientbound Update Recipes packet.
pub const UPDATE_RECIPES_PACKET_ID: i32 = 0x6D;

/// Packet ID for the serverbound Player Digging packet.
pub const PLAYER_DIGGING_PACKET_ID: i32 = 0x1D;

/// Packet ID for the serverbound Use Item On (Place Block) packet.
pub const USE_ITEM_ON_PACKET_ID: i32 = 0x31;

/// Packet ID for the clientbound Block Update packet.
pub const BLOCK_UPDATE_PACKET_ID: i32 = 0x0A;

/// Packet ID for the clientbound Acknowledge Block Change packet.
pub const ACKNOWLEDGE_BLOCK_CHANGE_PACKET_ID: i32 = 0x06;

/// Packet ID for the clientbound Move Entity (Pos) packet.
pub const MOVE_ENTITY_POS_PACKET_ID: i32 = 0x29;

/// Packet ID for the clientbound Move Entity (Pos+Rot) packet.
pub const MOVE_ENTITY_POS_ROT_PACKET_ID: i32 = 0x2A;

/// Packet ID for the clientbound Move Entity (Rot) packet.
pub const MOVE_ENTITY_ROT_PACKET_ID: i32 = 0x2B;

/// Packet ID for the clientbound Entity Teleport packet.
pub const ENTITY_TELEPORT_PACKET_ID: i32 = 0x57;

/// Packet ID for the clientbound Set Container Content packet.
pub const SET_CONTAINER_CONTENT_PACKET_ID: i32 = 0x12;

/// Packet ID for the clientbound Set Container Slot packet.
pub const SET_CONTAINER_SLOT_PACKET_ID: i32 = 0x14;

/// Packet ID for the serverbound Set Held Item (Carried Item) packet.
pub const SET_HELD_ITEM_SERVERBOUND_PACKET_ID: i32 = 0x28;

/// Packet ID for the serverbound Set Creative Mode Slot packet.
pub const SET_CREATIVE_MODE_SLOT_PACKET_ID: i32 = 0x2B;

/// Packet ID for the clientbound Set Health packet.
pub const SET_HEALTH_PACKET_ID: i32 = 0x5B;

/// Packet ID for the clientbound Respawn packet.
pub const RESPAWN_PACKET_ID: i32 = 0x45;

/// Packet ID for the serverbound Client Status packet.
pub const CLIENT_STATUS_PACKET_ID: i32 = 0x08;

/// Maximum number of slots in a single container content packet.
pub const MAX_CONTAINER_SLOTS: usize = 256;

/// Maximum size of an item NBT blob.
pub const MAX_ITEM_NBT_SIZE: usize = 65536;

/// Packet ID for the serverbound Chat Message packet.
pub const CHAT_MESSAGE_SERVERBOUND_PACKET_ID: i32 = 0x01;

/// Packet ID for the clientbound System Chat Message packet.
pub const SYSTEM_CHAT_MESSAGE_PACKET_ID: i32 = 0x5D;

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
    /// A compression error occurred.
    Compression(crate::compression::CompressionError),
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
            Self::Compression(error) => write!(formatter, "compression error: {error}"),
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
            Self::Compression(error) => Some(error),
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

impl From<crate::compression::CompressionError> for PlayError {
    fn from(error: crate::compression::CompressionError) -> Self {
        match error {
            crate::compression::CompressionError::Incomplete => Self::Incomplete,
            other => Self::Compression(other),
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
    /// Serverbound Confirm Teleportation (Play `0x00`).
    ConfirmTeleportation(ConfirmTeleportation),
    /// Serverbound Client Information (Play `0x08`).
    ClientInformation(ClientInformation),
    /// Clientbound Plugin Message (Play `0x17`).
    PluginMessageClientbound(PluginMessageClientbound),
    /// Clientbound Change Difficulty (Play `0x0C`).
    ChangeDifficulty(ChangeDifficulty),
    /// Clientbound Player Abilities (Play `0x34`).
    PlayerAbilities(PlayerAbilities),
    /// Clientbound Set Held Item (Play `0x4D`).
    SetHeldItem(SetHeldItem),
    /// Clientbound Entity Event (Play `0x1C`).
    EntityEvent(EntityEvent),
    /// Clientbound Game Event (Play `0x1F`).
    GameEvent(GameEvent),
    /// Clientbound Set Default Spawn Position (Play `0x50`).
    SetDefaultSpawnPosition(SetDefaultSpawnPosition),
    /// Clientbound Set Center Chunk (Play `0x4E`).
    SetCenterChunk(SetCenterChunk),
    /// Clientbound Unload Chunk (Play `0x1E`).
    UnloadChunk(UnloadChunk),
    /// Clientbound Combat Death (Play `0x38`).
    CombatDeath(CombatDeath),
    /// Clientbound Set Block Destroy Stage (Play `0x07`).
    SetBlockDestroyStage(SetBlockDestroyStage),
    /// Clientbound Set Render Distance (Play `0x4F`).
    SetRenderDistance(SetRenderDistance),
    /// Clientbound Set Simulation Distance (Play `0x5C`).
    SetSimulationDistance(SetSimulationDistance),
    /// Clientbound Player Info Update (Play `0x3A`).
    PlayerInfoUpdate(PlayerInfoUpdate),
    /// Clientbound Player Info Remove (Play `0x39`).
    PlayerInfoRemove(PlayerInfoRemove),
    /// Clientbound Spawn Player (Play `0x03`).
    SpawnPlayer(SpawnPlayer),
    /// Clientbound Remove Entities (Play `0x3E`).
    RemoveEntities(RemoveEntities),
    /// Serverbound Player Digging (Play `0x1D`).
    PlayerDigging(PlayerDigging),
    /// Serverbound Use Item On (Play `0x31`).
    UseItemOn(UseItemOn),
    /// Clientbound Block Update (Play `0x0A`).
    BlockUpdate(BlockUpdate),
    /// Clientbound Acknowledge Block Change (Play `0x06`).
    AcknowledgeBlockChange(AcknowledgeBlockChange),
    /// Clientbound Move Entity (Pos) (Play `0x29`).
    MoveEntityPos(MoveEntityPos),
    /// Clientbound Move Entity (Pos+Rot) (Play `0x2A`).
    MoveEntityPosRot(MoveEntityPosRot),
    /// Clientbound Move Entity (Rot) (Play `0x2B`).
    MoveEntityRot(MoveEntityRot),
    /// Clientbound Entity Teleport (Play `0x57`).
    EntityTeleport(EntityTeleport),
    /// Clientbound Set Container Content (Play `0x12`).
    SetContainerContent(SetContainerContent),
    /// Clientbound Set Container Slot (Play `0x14`).
    SetContainerSlot(SetContainerSlot),
    /// Serverbound Set Held Item (Play `0x28`).
    SetHeldItemServerbound(SetHeldItemServerbound),
    /// Serverbound Set Creative Mode Slot (Play `0x2B`).
    SetCreativeModeSlot(SetCreativeModeSlot),
    /// Serverbound Chat Message (Play `0x01`).
    ChatMessageServerbound(ChatMessageServerbound),
    /// Clientbound System Chat Message (Play `0x5D`).
    SystemChatMessage(SystemChatMessage),
    /// Clientbound Set Health (Play `0x5B`).
    SetHealth(SetHealth),
    /// Clientbound Respawn (Play `0x45`).
    Respawn(Respawn),
    /// Serverbound Client Status (Play `0x08`).
    ClientStatus(ClientStatus),
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
    /// Whether a death location is present.
    pub has_death_location: bool,
    /// Death dimension name (only if has_death_location).
    pub death_dimension_name: String,
    /// Death block position (only if has_death_location).
    pub death_location: (i32, i32, i32),
    /// Portal cooldown in ticks.
    pub portal_cooldown: i32,
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
    /// The light data blob (opaque bytes: masks + light arrays).
    pub light_data: Vec<u8>,
}

/// Serverbound Confirm Teleportation packet (Play `0x00`).
///
/// Sent by the client to confirm a Synchronize Player Position packet.
/// The teleport ID must match the one sent by the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConfirmTeleportation {
    /// The teleport ID to confirm.
    pub teleport_id: i32,
}

/// Chat mode setting for Client Information.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatMode {
    /// Full chat (enabled).
    Full,
    /// System chat only (commands, system messages).
    System,
    /// Chat hidden.
    Hidden,
}

impl ChatMode {
    /// Converts to wire value.
    pub fn to_wire(self) -> i32 {
        match self {
            Self::Full => 0,
            Self::System => 1,
            Self::Hidden => 2,
        }
    }

    /// Converts from wire value.
    pub fn from_wire(value: i32) -> Option<Self> {
        match value {
            0 => Some(Self::Full),
            1 => Some(Self::System),
            2 => Some(Self::Hidden),
            _ => None,
        }
    }
}

/// Player's main hand setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MainHand {
    /// Left hand.
    Left,
    /// Right hand.
    Right,
}

impl MainHand {
    /// Converts to wire value.
    pub fn to_wire(self) -> i32 {
        match self {
            Self::Left => 0,
            Self::Right => 1,
        }
    }

    /// Converts from wire value.
    pub fn from_wire(value: i32) -> Option<Self> {
        match value {
            0 => Some(Self::Left),
            1 => Some(Self::Right),
            _ => None,
        }
    }
}

/// Serverbound Client Information packet (Play `0x08`).
///
/// Sent by the client during the Play state to communicate settings
/// like locale, view distance, chat mode, and skin parts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientInformation {
    /// The client's locale (e.g. "en_US").
    pub locale: String,
    /// The client's view distance (in chunks, 2-32).
    pub view_distance: u8,
    /// The client's chat mode.
    pub chat_mode: ChatMode,
    /// Whether chat colors are enabled.
    pub chat_colors: bool,
    /// The displayed skin parts bitmask (7 bits).
    pub displayed_skin_parts: u8,
    /// The player's main hand.
    pub main_hand: MainHand,
    /// Whether text filtering is enabled.
    pub enable_text_filtering: bool,
    /// Whether the player wants to appear in the player sample list.
    pub allow_server_listings: bool,
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

    // Registry codec is raw NBT on the wire (self-delimiting; NOT length-prefixed).
    if packet.registry_codec.len() > MAX_REGISTRY_CODEC_SIZE {
        return Err(PlayError::Codec(CodecError::StringTooLong));
    }
    body.extend_from_slice(&packet.registry_codec);

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
    encode_bool(packet.has_death_location, &mut body);
    if packet.has_death_location {
        encode_string(
            &packet.death_dimension_name,
            MAX_IDENTIFIER_LENGTH,
            &mut body,
        )
        .map_err(PlayError::from)?;
        encode_position(
            packet.death_location.0,
            packet.death_location.1,
            packet.death_location.2,
            &mut body,
        );
    }
    encode_var_int(packet.portal_cooldown, &mut body);

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

    // Registry codec is raw NBT (self-delimiting).
    let registry_codec = read_nbt_compound(&mut body).map_err(|error| {
        *input = source;
        PlayError::from(error)
    })?;
    if registry_codec.len() > MAX_REGISTRY_CODEC_SIZE {
        *input = source;
        return Err(PlayError::Codec(CodecError::StringTooLong));
    }

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
    let has_death_location = decode_bool(&mut body).map_err(|error| {
        *input = source;
        PlayError::from(error)
    })?;
    let (death_dimension_name, death_location) = if has_death_location {
        let name = decode_string(&mut body, MAX_IDENTIFIER_LENGTH).map_err(|error| {
            *input = source;
            PlayError::from(error)
        })?;
        let pos = decode_position(&mut body).map_err(|error| {
            *input = source;
            PlayError::from(error)
        })?;
        (name.to_string(), pos)
    } else {
        (String::new(), (0, 0, 0))
    };
    let portal_cooldown = decode_var_int(&mut body).map_err(|error| {
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
            registry_codec,
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
            has_death_location,
            death_dimension_name,
            death_location,
            portal_cooldown,
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
    encode_f64(packet.x, &mut body);
    encode_f64(packet.y, &mut body);
    encode_f64(packet.z, &mut body);
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
    let x = decode_f64(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let y = decode_f64(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let z = decode_f64(&mut body).map_err(|e| {
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
    encode_f64(packet.x, &mut body);
    encode_f64(packet.y, &mut body);
    encode_f64(packet.z, &mut body);
    encode_f32(packet.yaw, &mut body);
    encode_f32(packet.pitch, &mut body);
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
    let x = decode_f64(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let y = decode_f64(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let z = decode_f64(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let yaw = decode_f32(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let pitch = decode_f32(&mut body).map_err(|e| {
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
    encode_f32(packet.yaw, &mut body);
    encode_f32(packet.pitch, &mut body);
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
    let yaw = decode_f32(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let pitch = decode_f32(&mut body).map_err(|e| {
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
    encode_f64(packet.x, &mut body);
    encode_f64(packet.y, &mut body);
    encode_f64(packet.z, &mut body);
    encode_f32(packet.yaw, &mut body);
    encode_f32(packet.pitch, &mut body);
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
    let x = decode_f64(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let y = decode_f64(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let z = decode_f64(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let yaw = decode_f32(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let pitch = decode_f32(&mut body).map_err(|e| {
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
// Confirm Teleportation (serverbound Play 0x00)
// ---------------------------------------------------------------------------

/// Encodes a Confirm Teleportation packet (serverbound Play `0x00`).
pub fn encode_confirm_teleportation(
    packet: &ConfirmTeleportation,
    max_frame_length: usize,
    output: &mut Vec<u8>,
) -> Result<(), PlayError> {
    let mut body = Vec::new();
    encode_var_int(packet.teleport_id, &mut body);
    encode_frame(
        CONFIRM_TELEPORTATION_PACKET_ID,
        &body,
        max_frame_length,
        output,
    )
    .map_err(PlayError::from)?;
    Ok(())
}

/// Decodes a Confirm Teleportation packet (serverbound Play `0x00`).
pub fn decode_confirm_teleportation(
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

    if frame.packet_id != CONFIRM_TELEPORTATION_PACKET_ID {
        *input = source;
        return Err(PlayError::WrongPacketId {
            received: frame.packet_id,
            expected: CONFIRM_TELEPORTATION_PACKET_ID,
        });
    }

    let mut body = frame.payload;
    let teleport_id = decode_var_int(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;

    if !body.is_empty() {
        *input = source;
        return Err(PlayError::TrailingBytes { count: body.len() });
    }

    Ok(PlayDecodeOutcome::Complete(
        PlayPacket::ConfirmTeleportation(ConfirmTeleportation { teleport_id }),
    ))
}

// ---------------------------------------------------------------------------
// Client Information (serverbound Play 0x08)
// ---------------------------------------------------------------------------

/// Encodes a Client Information packet (serverbound Play `0x08`).
pub fn encode_client_information(
    packet: &ClientInformation,
    max_frame_length: usize,
    output: &mut Vec<u8>,
) -> Result<(), PlayError> {
    let mut body = Vec::new();
    encode_string(&packet.locale, MAX_CHAT_COMPONENT_LENGTH, &mut body).map_err(PlayError::from)?;
    encode_u8(packet.view_distance, &mut body);
    encode_var_int(packet.chat_mode.to_wire(), &mut body);
    encode_bool(packet.chat_colors, &mut body);
    encode_u8(packet.displayed_skin_parts, &mut body);
    encode_var_int(packet.main_hand.to_wire(), &mut body);
    encode_bool(packet.enable_text_filtering, &mut body);
    encode_bool(packet.allow_server_listings, &mut body);
    encode_frame(
        CLIENT_INFORMATION_PACKET_ID,
        &body,
        max_frame_length,
        output,
    )
    .map_err(PlayError::from)?;
    Ok(())
}

/// Decodes a Client Information packet (serverbound Play `0x08`).
pub fn decode_client_information(
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

    if frame.packet_id != CLIENT_INFORMATION_PACKET_ID {
        *input = source;
        return Err(PlayError::WrongPacketId {
            received: frame.packet_id,
            expected: CLIENT_INFORMATION_PACKET_ID,
        });
    }

    let mut body = frame.payload;
    let locale = decode_string(&mut body, MAX_CHAT_COMPONENT_LENGTH).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let view_distance = decode_u8(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let chat_mode_raw = decode_var_int(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let chat_mode = ChatMode::from_wire(chat_mode_raw).ok_or_else(|| {
        *input = source;
        PlayError::Codec(CodecError::InvalidBoolean)
    })?;
    let chat_colors = decode_bool(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let displayed_skin_parts = decode_u8(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let main_hand_raw = decode_var_int(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let main_hand = MainHand::from_wire(main_hand_raw).ok_or_else(|| {
        *input = source;
        PlayError::Codec(CodecError::InvalidBoolean)
    })?;
    let enable_text_filtering = decode_bool(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let allow_server_listings = decode_bool(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;

    if !body.is_empty() {
        *input = source;
        return Err(PlayError::TrailingBytes { count: body.len() });
    }

    Ok(PlayDecodeOutcome::Complete(PlayPacket::ClientInformation(
        ClientInformation {
            locale: locale.to_string(),
            view_distance,
            chat_mode,
            chat_colors,
            displayed_skin_parts,
            main_hand,
            enable_text_filtering,
            allow_server_listings,
        },
    )))
}

// ---------------------------------------------------------------------------
// Join sequence packets (clientbound Play, protocol 763)
// ---------------------------------------------------------------------------

/// Clientbound Plugin Message (Play `0x17`).
///
/// Carries custom payload data on a registered or brand channel.
/// The brand channel `minecraft:brand` carries the server brand string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginMessageClientbound {
    /// The channel identifier (e.g. `minecraft:brand`).
    pub channel: String,
    /// The payload data.
    pub data: Vec<u8>,
}

/// Clientbound Change Difficulty packet (Play `0x0C`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChangeDifficulty {
    /// The difficulty (0=Peaceful, 1=Easy, 2=Normal, 3=Hard).
    pub difficulty: u8,
    /// Whether the difficulty is locked.
    pub locked: bool,
}

/// Clientbound Player Abilities packet (Play `0x34`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlayerAbilities {
    /// Flags bitmask: bit 0 = invulnerable, bit 1 = flying,
    /// bit 2 = allow flying, bit 3 = creative mode.
    pub flags: u8,
    /// Flying speed (0.0 E.0, typically 0.05).
    pub flying_speed: f32,
    /// Field of view modifier (0.0 E.0, typically 0.1).
    pub fov_modifier: f32,
}

/// Clientbound Set Held Item packet (Play `0x4D`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetHeldItem {
    /// The hotbar slot (0 E).
    pub slot: u8,
}

/// Clientbound Entity Event (Entity Status) packet (Play `0x1C`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntityEvent {
    /// The entity ID.
    pub entity_id: i32,
    /// The entity status byte.
    pub entity_status: u8,
}

/// Clientbound Game Event packet (Play `0x1F`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GameEvent {
    /// The event type (e.g. 13 = Start Waiting for Level Chunks).
    pub event_type: u8,
    /// The event value (float, interpretation depends on event type).
    pub value: f32,
}

/// Clientbound Set Default Spawn Position packet (Play `0x50`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SetDefaultSpawnPosition {
    /// The spawn position.
    pub location: (i32, i32, i32),
    /// The spawn angle (in degrees).
    pub angle: f32,
}

/// Clientbound Set Center Chunk packet (Play `0x4E`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetCenterChunk {
    /// The chunk X coordinate.
    pub chunk_x: i32,
    /// The chunk Z coordinate.
    pub chunk_z: i32,
}

/// Clientbound Unload Chunk packet (Play `0x1E`).
///
/// Tells the client to unload a chunk column. It is legal to send this
/// packet even if the given chunk is not currently loaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnloadChunk {
    /// The chunk X coordinate.
    pub chunk_x: i32,
    /// The chunk Z coordinate.
    pub chunk_z: i32,
}

/// Clientbound Combat Death packet (Play `0x38`).
///
/// Sent to display the death screen with a death message.
/// The player ID should match the client's own entity ID.
#[derive(Debug, Clone, PartialEq)]
pub struct CombatDeath {
    /// The entity ID of the player that died.
    pub player_id: i32,
    /// The death message as a JSON chat component string.
    pub message: String,
}

/// Clientbound Set Block Destroy Stage packet (Play `0x07`).
///
/// Sent to display the block break animation. Destroy stage 0–9 sets
/// the animation; any other value removes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetBlockDestroyStage {
    /// The entity ID breaking the block (does not need to be a real entity).
    pub entity_id: i32,
    /// The block position.
    pub position: (i32, i32, i32),
    /// The destroy stage (0–9 to set, any other value to remove).
    pub destroy_stage: i8,
}

/// Clientbound Set Render Distance packet (Play `0x4F`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetRenderDistance {
    /// The view distance (in chunks).
    pub view_distance: i32,
}

/// Clientbound Set Simulation Distance packet (Play `0x5C`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetSimulationDistance {
    /// The simulation distance (in chunks).
    pub simulation_distance: i32,
}

/// Encodes a Plugin Message (clientbound Play `0x17`).
pub fn encode_plugin_message_clientbound(
    packet: &PluginMessageClientbound,
    max_frame_length: usize,
    output: &mut Vec<u8>,
) -> Result<(), PlayError> {
    let mut body = Vec::new();
    encode_string(&packet.channel, MAX_CHAT_COMPONENT_LENGTH, &mut body)
        .map_err(PlayError::from)?;
    body.extend_from_slice(&packet.data);
    encode_frame(
        PLUGIN_MESSAGE_CLIENTBOUND_PACKET_ID,
        &body,
        max_frame_length,
        output,
    )
    .map_err(PlayError::from)?;
    Ok(())
}

/// Decodes a Plugin Message (clientbound Play `0x17`).
pub fn decode_plugin_message_clientbound(
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
    if frame.packet_id != PLUGIN_MESSAGE_CLIENTBOUND_PACKET_ID {
        *input = source;
        return Err(PlayError::WrongPacketId {
            received: frame.packet_id,
            expected: PLUGIN_MESSAGE_CLIENTBOUND_PACKET_ID,
        });
    }
    let mut body = frame.payload;
    let channel = decode_string(&mut body, MAX_CHAT_COMPONENT_LENGTH).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    Ok(PlayDecodeOutcome::Complete(
        PlayPacket::PluginMessageClientbound(PluginMessageClientbound {
            channel: channel.to_string(),
            data: body.to_vec(),
        }),
    ))
}

/// Encodes a Change Difficulty packet (clientbound Play `0x0C`).
pub fn encode_change_difficulty(
    packet: &ChangeDifficulty,
    max_frame_length: usize,
    output: &mut Vec<u8>,
) -> Result<(), PlayError> {
    let mut body = Vec::new();
    encode_u8(packet.difficulty, &mut body);
    encode_bool(packet.locked, &mut body);
    encode_frame(CHANGE_DIFFICULTY_PACKET_ID, &body, max_frame_length, output)
        .map_err(PlayError::from)?;
    Ok(())
}

/// Decodes a Change Difficulty packet (clientbound Play `0x0C`).
pub fn decode_change_difficulty(
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
    if frame.packet_id != CHANGE_DIFFICULTY_PACKET_ID {
        *input = source;
        return Err(PlayError::WrongPacketId {
            received: frame.packet_id,
            expected: CHANGE_DIFFICULTY_PACKET_ID,
        });
    }
    let mut body = frame.payload;
    let difficulty = decode_u8(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let locked = decode_bool(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    if !body.is_empty() {
        *input = source;
        return Err(PlayError::TrailingBytes { count: body.len() });
    }
    Ok(PlayDecodeOutcome::Complete(PlayPacket::ChangeDifficulty(
        ChangeDifficulty { difficulty, locked },
    )))
}

/// Encodes a Player Abilities packet (clientbound Play `0x34`).
pub fn encode_player_abilities(
    packet: &PlayerAbilities,
    max_frame_length: usize,
    output: &mut Vec<u8>,
) -> Result<(), PlayError> {
    let mut body = Vec::new();
    encode_u8(packet.flags, &mut body);
    encode_f32(packet.flying_speed, &mut body);
    encode_f32(packet.fov_modifier, &mut body);
    encode_frame(PLAYER_ABILITIES_PACKET_ID, &body, max_frame_length, output)
        .map_err(PlayError::from)?;
    Ok(())
}

/// Encodes an empty Update Tags packet (clientbound Play `0x68`).
///
/// Vanilla clients expect tags for `minecraft:block`, `minecraft:item`,
/// `minecraft:fluid`, `minecraft:entity_type`, and `minecraft:game_event`.
/// Empty tag lists are accepted and unblock join; gameplay that depends
/// on tags remains stubbed.
pub fn encode_update_tags_empty(
    max_frame_length: usize,
    output: &mut Vec<u8>,
) -> Result<(), PlayError> {
    const REGISTRIES: &[&str] = &[
        "minecraft:block",
        "minecraft:item",
        "minecraft:fluid",
        "minecraft:entity_type",
        "minecraft:game_event",
    ];
    let mut body = Vec::new();
    encode_var_int(REGISTRIES.len() as i32, &mut body);
    for registry in REGISTRIES {
        encode_string(registry, MAX_IDENTIFIER_LENGTH, &mut body).map_err(PlayError::from)?;
        encode_var_int(0, &mut body); // zero tags in this registry
    }
    encode_frame(UPDATE_TAGS_PACKET_ID, &body, max_frame_length, output)
        .map_err(PlayError::from)?;
    Ok(())
}

/// Encodes an empty Update Recipes packet (clientbound Play `0x6D`).
pub fn encode_update_recipes_empty(
    max_frame_length: usize,
    output: &mut Vec<u8>,
) -> Result<(), PlayError> {
    let mut body = Vec::new();
    encode_var_int(0, &mut body); // zero recipes
    encode_frame(UPDATE_RECIPES_PACKET_ID, &body, max_frame_length, output)
        .map_err(PlayError::from)?;
    Ok(())
}

/// Decodes a Player Abilities packet (clientbound Play `0x34`).
pub fn decode_player_abilities(
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
    if frame.packet_id != PLAYER_ABILITIES_PACKET_ID {
        *input = source;
        return Err(PlayError::WrongPacketId {
            received: frame.packet_id,
            expected: PLAYER_ABILITIES_PACKET_ID,
        });
    }
    let mut body = frame.payload;
    let flags = decode_u8(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let flying_speed = decode_f32(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let fov_modifier = decode_f32(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    if !body.is_empty() {
        *input = source;
        return Err(PlayError::TrailingBytes { count: body.len() });
    }
    Ok(PlayDecodeOutcome::Complete(PlayPacket::PlayerAbilities(
        PlayerAbilities {
            flags,
            flying_speed,
            fov_modifier,
        },
    )))
}

/// Encodes a Set Held Item packet (clientbound Play `0x4D`).
pub fn encode_set_held_item(
    packet: &SetHeldItem,
    max_frame_length: usize,
    output: &mut Vec<u8>,
) -> Result<(), PlayError> {
    let mut body = Vec::new();
    encode_u8(packet.slot, &mut body);
    encode_frame(SET_HELD_ITEM_PACKET_ID, &body, max_frame_length, output)
        .map_err(PlayError::from)?;
    Ok(())
}

/// Decodes a Set Held Item packet (clientbound Play `0x4D`).
pub fn decode_set_held_item(
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
    if frame.packet_id != SET_HELD_ITEM_PACKET_ID {
        *input = source;
        return Err(PlayError::WrongPacketId {
            received: frame.packet_id,
            expected: SET_HELD_ITEM_PACKET_ID,
        });
    }
    let mut body = frame.payload;
    let slot = decode_u8(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    if !body.is_empty() {
        *input = source;
        return Err(PlayError::TrailingBytes { count: body.len() });
    }
    Ok(PlayDecodeOutcome::Complete(PlayPacket::SetHeldItem(
        SetHeldItem { slot },
    )))
}

/// Encodes an Entity Event packet (clientbound Play `0x1C`).
pub fn encode_entity_event(
    packet: &EntityEvent,
    max_frame_length: usize,
    output: &mut Vec<u8>,
) -> Result<(), PlayError> {
    let mut body = Vec::new();
    encode_i32(packet.entity_id, &mut body);
    encode_u8(packet.entity_status, &mut body);
    encode_frame(ENTITY_EVENT_PACKET_ID, &body, max_frame_length, output)
        .map_err(PlayError::from)?;
    Ok(())
}

/// Decodes an Entity Event packet (clientbound Play `0x1C`).
pub fn decode_entity_event(
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
    if frame.packet_id != ENTITY_EVENT_PACKET_ID {
        *input = source;
        return Err(PlayError::WrongPacketId {
            received: frame.packet_id,
            expected: ENTITY_EVENT_PACKET_ID,
        });
    }
    let mut body = frame.payload;
    let entity_id = decode_i32(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let entity_status = decode_u8(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    if !body.is_empty() {
        *input = source;
        return Err(PlayError::TrailingBytes { count: body.len() });
    }
    Ok(PlayDecodeOutcome::Complete(PlayPacket::EntityEvent(
        EntityEvent {
            entity_id,
            entity_status,
        },
    )))
}

/// Encodes a Game Event packet (clientbound Play `0x1F`).
pub fn encode_game_event(
    packet: &GameEvent,
    max_frame_length: usize,
    output: &mut Vec<u8>,
) -> Result<(), PlayError> {
    let mut body = Vec::new();
    encode_u8(packet.event_type, &mut body);
    encode_f32(packet.value, &mut body);
    encode_frame(GAME_EVENT_PACKET_ID, &body, max_frame_length, output).map_err(PlayError::from)?;
    Ok(())
}

/// Decodes a Game Event packet (clientbound Play `0x1F`).
pub fn decode_game_event(
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
    if frame.packet_id != GAME_EVENT_PACKET_ID {
        *input = source;
        return Err(PlayError::WrongPacketId {
            received: frame.packet_id,
            expected: GAME_EVENT_PACKET_ID,
        });
    }
    let mut body = frame.payload;
    let event_type = decode_u8(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let value = decode_f32(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    if !body.is_empty() {
        *input = source;
        return Err(PlayError::TrailingBytes { count: body.len() });
    }
    Ok(PlayDecodeOutcome::Complete(PlayPacket::GameEvent(
        GameEvent { event_type, value },
    )))
}

/// Encodes a Set Default Spawn Position packet (clientbound Play `0x50`).
pub fn encode_set_default_spawn_position(
    packet: &SetDefaultSpawnPosition,
    max_frame_length: usize,
    output: &mut Vec<u8>,
) -> Result<(), PlayError> {
    let mut body = Vec::new();
    encode_position(
        packet.location.0,
        packet.location.1,
        packet.location.2,
        &mut body,
    );
    encode_f32(packet.angle, &mut body);
    encode_frame(
        SET_DEFAULT_SPAWN_POSITION_PACKET_ID,
        &body,
        max_frame_length,
        output,
    )
    .map_err(PlayError::from)?;
    Ok(())
}

/// Decodes a Set Default Spawn Position packet (clientbound Play `0x50`).
pub fn decode_set_default_spawn_position(
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
    if frame.packet_id != SET_DEFAULT_SPAWN_POSITION_PACKET_ID {
        *input = source;
        return Err(PlayError::WrongPacketId {
            received: frame.packet_id,
            expected: SET_DEFAULT_SPAWN_POSITION_PACKET_ID,
        });
    }
    let mut body = frame.payload;
    let (x, y, z) = decode_position(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let angle = decode_f32(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    if !body.is_empty() {
        *input = source;
        return Err(PlayError::TrailingBytes { count: body.len() });
    }
    Ok(PlayDecodeOutcome::Complete(
        PlayPacket::SetDefaultSpawnPosition(SetDefaultSpawnPosition {
            location: (x, y, z),
            angle,
        }),
    ))
}

/// Encodes a Set Center Chunk packet (clientbound Play `0x4E`).
pub fn encode_set_center_chunk(
    packet: &SetCenterChunk,
    max_frame_length: usize,
    output: &mut Vec<u8>,
) -> Result<(), PlayError> {
    let mut body = Vec::new();
    encode_var_int(packet.chunk_x, &mut body);
    encode_var_int(packet.chunk_z, &mut body);
    encode_frame(SET_CENTER_CHUNK_PACKET_ID, &body, max_frame_length, output)
        .map_err(PlayError::from)?;
    Ok(())
}

/// Decodes a Set Center Chunk packet (clientbound Play `0x4E`).
pub fn decode_set_center_chunk(
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
    if frame.packet_id != SET_CENTER_CHUNK_PACKET_ID {
        *input = source;
        return Err(PlayError::WrongPacketId {
            received: frame.packet_id,
            expected: SET_CENTER_CHUNK_PACKET_ID,
        });
    }
    let mut body = frame.payload;
    let chunk_x = decode_var_int(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let chunk_z = decode_var_int(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    if !body.is_empty() {
        *input = source;
        return Err(PlayError::TrailingBytes { count: body.len() });
    }
    Ok(PlayDecodeOutcome::Complete(PlayPacket::SetCenterChunk(
        SetCenterChunk { chunk_x, chunk_z },
    )))
}

/// Encodes an Unload Chunk packet (clientbound Play `0x1E`).
///
/// Fields are written as Int (big-endian i32), X first then Z.
pub fn encode_unload_chunk(
    packet: &UnloadChunk,
    max_frame_length: usize,
    output: &mut Vec<u8>,
) -> Result<(), PlayError> {
    let mut body = Vec::with_capacity(8);
    body.extend_from_slice(&packet.chunk_x.to_be_bytes());
    body.extend_from_slice(&packet.chunk_z.to_be_bytes());
    encode_frame(UNLOAD_CHUNK_PACKET_ID, &body, max_frame_length, output)
        .map_err(PlayError::from)?;
    Ok(())
}

/// Decodes an Unload Chunk packet (clientbound Play `0x1E`).
pub fn decode_unload_chunk(
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
    if frame.packet_id != UNLOAD_CHUNK_PACKET_ID {
        *input = source;
        return Err(PlayError::WrongPacketId {
            received: frame.packet_id,
            expected: UNLOAD_CHUNK_PACKET_ID,
        });
    }
    let body = &frame.payload;
    if body.len() < 8 {
        *input = source;
        return Err(PlayError::TrailingBytes { count: 0 });
    }
    let chunk_x = i32::from_be_bytes(
        body[0..4]
            .try_into()
            .map_err(|_| PlayError::TrailingBytes { count: 0 })?,
    );
    let chunk_z = i32::from_be_bytes(
        body[4..8]
            .try_into()
            .map_err(|_| PlayError::TrailingBytes { count: 0 })?,
    );
    let trailing = &body[8..];
    if !trailing.is_empty() {
        *input = source;
        return Err(PlayError::TrailingBytes {
            count: trailing.len(),
        });
    }
    Ok(PlayDecodeOutcome::Complete(PlayPacket::UnloadChunk(
        UnloadChunk { chunk_x, chunk_z },
    )))
}

/// Encodes a Combat Death packet (clientbound Play `0x38`).
pub fn encode_combat_death(
    packet: &CombatDeath,
    max_frame_length: usize,
    output: &mut Vec<u8>,
) -> Result<(), PlayError> {
    let mut body = Vec::new();
    encode_var_int(packet.player_id, &mut body);
    encode_string(&packet.message, 32767, &mut body).map_err(PlayError::Codec)?;
    encode_frame(COMBAT_DEATH_PACKET_ID, &body, max_frame_length, output)
        .map_err(PlayError::from)?;
    Ok(())
}

/// Decodes a Combat Death packet (clientbound Play `0x38`).
pub fn decode_combat_death(
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
    if frame.packet_id != COMBAT_DEATH_PACKET_ID {
        *input = source;
        return Err(PlayError::WrongPacketId {
            received: frame.packet_id,
            expected: COMBAT_DEATH_PACKET_ID,
        });
    }
    let mut body = frame.payload;
    let player_id = decode_var_int(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let message = decode_string(&mut body, 32767).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    if !body.is_empty() {
        *input = source;
        return Err(PlayError::TrailingBytes { count: body.len() });
    }
    Ok(PlayDecodeOutcome::Complete(PlayPacket::CombatDeath(
        CombatDeath {
            player_id,
            message: message.to_string(),
        },
    )))
}

/// Encodes a Set Block Destroy Stage packet (clientbound Play `0x07`).
pub fn encode_set_block_destroy_stage(
    packet: &SetBlockDestroyStage,
    max_frame_length: usize,
    output: &mut Vec<u8>,
) -> Result<(), PlayError> {
    let mut body = Vec::new();
    encode_var_int(packet.entity_id, &mut body);
    encode_position(
        packet.position.0,
        packet.position.1,
        packet.position.2,
        &mut body,
    );
    encode_i8(packet.destroy_stage, &mut body);
    encode_frame(
        SET_BLOCK_DESTROY_STAGE_PACKET_ID,
        &body,
        max_frame_length,
        output,
    )
    .map_err(PlayError::from)?;
    Ok(())
}

/// Decodes a Set Block Destroy Stage packet (clientbound Play `0x07`).
pub fn decode_set_block_destroy_stage(
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
    if frame.packet_id != SET_BLOCK_DESTROY_STAGE_PACKET_ID {
        *input = source;
        return Err(PlayError::WrongPacketId {
            received: frame.packet_id,
            expected: SET_BLOCK_DESTROY_STAGE_PACKET_ID,
        });
    }
    let mut body = frame.payload;
    let entity_id = decode_var_int(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let position = decode_position(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let destroy_stage = decode_i8(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    if !body.is_empty() {
        *input = source;
        return Err(PlayError::TrailingBytes { count: body.len() });
    }
    Ok(PlayDecodeOutcome::Complete(
        PlayPacket::SetBlockDestroyStage(SetBlockDestroyStage {
            entity_id,
            position,
            destroy_stage,
        }),
    ))
}

/// Encodes a Set Render Distance packet (clientbound Play `0x4F`).
pub fn encode_set_render_distance(
    packet: &SetRenderDistance,
    max_frame_length: usize,
    output: &mut Vec<u8>,
) -> Result<(), PlayError> {
    let mut body = Vec::new();
    encode_var_int(packet.view_distance, &mut body);
    encode_frame(
        SET_RENDER_DISTANCE_PACKET_ID,
        &body,
        max_frame_length,
        output,
    )
    .map_err(PlayError::from)?;
    Ok(())
}

/// Decodes a Set Render Distance packet (clientbound Play `0x4F`).
pub fn decode_set_render_distance(
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
    if frame.packet_id != SET_RENDER_DISTANCE_PACKET_ID {
        *input = source;
        return Err(PlayError::WrongPacketId {
            received: frame.packet_id,
            expected: SET_RENDER_DISTANCE_PACKET_ID,
        });
    }
    let mut body = frame.payload;
    let view_distance = decode_var_int(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    if !body.is_empty() {
        *input = source;
        return Err(PlayError::TrailingBytes { count: body.len() });
    }
    Ok(PlayDecodeOutcome::Complete(PlayPacket::SetRenderDistance(
        SetRenderDistance { view_distance },
    )))
}

/// Encodes a Set Simulation Distance packet (clientbound Play `0x5C`).
pub fn encode_set_simulation_distance(
    packet: &SetSimulationDistance,
    max_frame_length: usize,
    output: &mut Vec<u8>,
) -> Result<(), PlayError> {
    let mut body = Vec::new();
    encode_var_int(packet.simulation_distance, &mut body);
    encode_frame(
        SET_SIMULATION_DISTANCE_PACKET_ID,
        &body,
        max_frame_length,
        output,
    )
    .map_err(PlayError::from)?;
    Ok(())
}

/// Decodes a Set Simulation Distance packet (clientbound Play `0x5C`).
pub fn decode_set_simulation_distance(
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
    if frame.packet_id != SET_SIMULATION_DISTANCE_PACKET_ID {
        *input = source;
        return Err(PlayError::WrongPacketId {
            received: frame.packet_id,
            expected: SET_SIMULATION_DISTANCE_PACKET_ID,
        });
    }
    let mut body = frame.payload;
    let simulation_distance = decode_var_int(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    if !body.is_empty() {
        *input = source;
        return Err(PlayError::TrailingBytes { count: body.len() });
    }
    Ok(PlayDecodeOutcome::Complete(
        PlayPacket::SetSimulationDistance(SetSimulationDistance {
            simulation_distance,
        }),
    ))
}

// ---------------------------------------------------------------------------
// Multiplayer packets (clientbound Play, protocol 763)
// ---------------------------------------------------------------------------

/// Bit flags for Player Info Update actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayerInfoActions(pub u8);

impl PlayerInfoActions {
    /// Add player action (bit 0).
    pub const ADD_PLAYER: u8 = 0x01;
    /// Update gamemode action (bit 1).
    pub const UPDATE_GAMEMODE: u8 = 0x02;
    /// Update listed action (bit 2).
    pub const UPDATE_LISTED: u8 = 0x04;
    /// Update latency action (bit 3).
    pub const UPDATE_LATENCY: u8 = 0x08;
    /// Update display name action (bit 4).
    pub const UPDATE_DISPLAY_NAME: u8 = 0x10;

    /// Creates action flags with the given bits set.
    pub const fn new(bits: u8) -> Self {
        Self(bits)
    }

    /// Returns whether the given action bit is set.
    pub fn has(self, bit: u8) -> bool {
        self.0 & bit != 0
    }
}

/// A single player info entry.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerInfoEntry {
    /// The player's UUID.
    pub uuid: Uuid,
    /// The player's username (only present if ADD_PLAYER action).
    pub name: String,
    /// Array of property (name, value, signature) tuples (only if ADD_PLAYER).
    pub properties: Vec<(String, String, Option<String>)>,
    /// The player's gamemode (only if ADD_PLAYER or UPDATE_GAMEMODE).
    pub gamemode: i32,
    /// Whether the player is listed (only if ADD_PLAYER or UPDATE_LISTED).
    pub listed: bool,
    /// The player's ping in milliseconds (only if ADD_PLAYER or UPDATE_LATENCY).
    pub latency: i32,
    /// The player's display name as JSON (only if UPDATE_DISPLAY_NAME).
    pub display_name: Option<String>,
}

/// Clientbound Player Info Update packet (Play `0x3A`).
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerInfoUpdate {
    /// Action bit flags.
    pub actions: PlayerInfoActions,
    /// The player info entries.
    pub entries: Vec<PlayerInfoEntry>,
}

/// Clientbound Player Info Remove packet (Play `0x39`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerInfoRemove {
    /// The UUIDs of players to remove.
    pub uuids: Vec<Uuid>,
}

/// Clientbound Spawn Player packet (Play `0x03`).
#[derive(Debug, Clone, PartialEq)]
pub struct SpawnPlayer {
    /// The entity ID.
    pub entity_id: i32,
    /// The player's UUID.
    pub uuid: Uuid,
    /// The X coordinate.
    pub x: f64,
    /// The Y coordinate.
    pub y: f64,
    /// The Z coordinate.
    pub z: f64,
    /// The yaw angle (degrees, 0-255).
    pub yaw: u8,
    /// The pitch angle (degrees, 0-255).
    pub pitch: u8,
}

/// Clientbound Remove Entities packet (Play `0x3E`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoveEntities {
    /// The entity IDs to remove.
    pub entity_ids: Vec<i32>,
}

/// Clientbound Move Entity (Pos) packet (Play `0x29`).
///
/// Sends a relative position update. The delta values are in 1/4096 of a block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MoveEntityPos {
    /// The entity ID.
    pub entity_id: i32,
    /// Change in X, in 1/4096 of a block.
    pub delta_x: i16,
    /// Change in Y, in 1/4096 of a block.
    pub delta_y: i16,
    /// Change in Z, in 1/4096 of a block.
    pub delta_z: i16,
    /// Whether the entity is on the ground.
    pub on_ground: bool,
}

/// Clientbound Move Entity (Pos+Rot) packet (Play `0x2A`).
///
/// Sends a relative position and rotation update. The delta values are in
/// 1/4096 of a block. Angles are in steps of 1/256 of a turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MoveEntityPosRot {
    /// The entity ID.
    pub entity_id: i32,
    /// Change in X, in 1/4096 of a block.
    pub delta_x: i16,
    /// Change in Y, in 1/4096 of a block.
    pub delta_y: i16,
    /// Change in Z, in 1/4096 of a block.
    pub delta_z: i16,
    /// The yaw angle (0-255, representing 0-360 degrees).
    pub yaw: u8,
    /// The pitch angle (0-255, representing 0-360 degrees).
    pub pitch: u8,
    /// Whether the entity is on the ground.
    pub on_ground: bool,
}

/// Clientbound Move Entity (Rot) packet (Play `0x2B`).
///
/// Sends a rotation-only update. Angles are in steps of 1/256 of a turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MoveEntityRot {
    /// The entity ID.
    pub entity_id: i32,
    /// The yaw angle (0-255, representing 0-360 degrees).
    pub yaw: u8,
    /// The pitch angle (0-255, representing 0-360 degrees).
    pub pitch: u8,
    /// Whether the entity is on the ground.
    pub on_ground: bool,
}

/// Clientbound Entity Teleport packet (Play `0x57`).
///
/// Sends an absolute position and rotation update. Used when the delta
/// exceeds the range of a relative move (more than ~8 blocks).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EntityTeleport {
    /// The entity ID.
    pub entity_id: i32,
    /// The absolute X coordinate.
    pub x: f64,
    /// The absolute Y coordinate.
    pub y: f64,
    /// The absolute Z coordinate.
    pub z: f64,
    /// The yaw angle (0-255, representing 0-360 degrees).
    pub yaw: u8,
    /// The pitch angle (0-255, representing 0-360 degrees).
    pub pitch: u8,
    /// Whether the entity is on the ground.
    pub on_ground: bool,
}

/// An item stack in an inventory slot.
///
/// In protocol 763, a slot is encoded as:
/// - `present` (bool): whether the slot contains an item
/// - If present:
///   - `item_id` (VarInt): the item ID
///   - `count` (i8): the stack size (1-127)
///   - `nbt` (optional NBT): item components/tags, or TAG_End (0x00) if none
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Slot {
    /// Whether the slot contains an item.
    pub present: bool,
    /// The item ID (only meaningful if `present` is true).
    pub item_id: i32,
    /// The stack size (only meaningful if `present` is true).
    pub count: i8,
    /// Raw NBT bytes for item components/tags (only meaningful if `present` is true).
    /// An empty Vec means no NBT (encoded as TAG_End = 0x00).
    pub nbt: Vec<u8>,
}

impl Slot {
    /// Creates an empty slot (no item).
    pub fn empty() -> Self {
        Self {
            present: false,
            item_id: 0,
            count: 0,
            nbt: Vec::new(),
        }
    }

    /// Creates a slot with an item and no NBT data.
    pub fn item(item_id: i32, count: i8) -> Self {
        Self {
            present: true,
            item_id,
            count,
            nbt: Vec::new(),
        }
    }

    /// Creates a slot with an item and raw NBT data.
    pub fn with_nbt(item_id: i32, count: i8, nbt: Vec<u8>) -> Self {
        Self {
            present: true,
            item_id,
            count,
            nbt,
        }
    }
}

impl Default for Slot {
    fn default() -> Self {
        Self::empty()
    }
}

/// Clientbound Set Container Content packet (Play `0x12`).
///
/// Replaces the entire contents of a container window (or the player
/// inventory when `window_id` is 0).
#[derive(Debug, Clone, PartialEq)]
pub struct SetContainerContent {
    /// The window ID (0 for player inventory).
    pub window_id: u8,
    /// Server-managed state ID for synchronization.
    pub state_id: i32,
    /// The slot data for all slots in the window.
    pub slots: Vec<Slot>,
    /// The item currently being dragged by the mouse cursor.
    pub carried_item: Slot,
}

/// Clientbound Set Container Slot packet (Play `0x14`).
///
/// Updates a single slot in a container window.
#[derive(Debug, Clone, PartialEq)]
pub struct SetContainerSlot {
    /// The window ID (0 for player inventory, -1 for cursor item, -2 for any player slot).
    pub window_id: i8,
    /// Server-managed state ID for synchronization.
    pub state_id: i32,
    /// The slot index to update.
    pub slot: i16,
    /// The new slot data.
    pub item: Slot,
}

/// Serverbound Set Held Item packet (Play `0x28`).
///
/// Sent when the player changes their hotbar slot selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetHeldItemServerbound {
    /// The hotbar slot (0-8) the player has selected.
    pub slot: i16,
}

/// Serverbound Set Creative Mode Slot packet (Play `0x2B`).
///
/// Sent in Creative mode when the player places or picks up an item
/// in the inventory. Slot -1 means dropping the item (spawn item entity).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetCreativeModeSlot {
    /// The slot index (-1 for drop/cursor).
    pub slot: i16,
    /// The item to place in the slot.
    pub item: Slot,
}

/// Clientbound Set Health packet (Play `0x5B`).
///
/// Sets the player's health, food, and food saturation. Health <= 0
/// triggers the death screen on the client.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SetHealth {
    /// The player's health (0 or less = dead, 20 = full HP).
    pub health: f32,
    /// The player's food level (0-20).
    pub food: i32,
    /// The player's food saturation (0.0 to 5.0).
    pub food_saturation: f32,
}

/// Clientbound Respawn packet (Play `0x45`).
///
/// Sent to respawn the player after death or change dimensions.
/// For a simple death/respawn in the same dimension, most fields can
/// be defaulted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Respawn {
    /// The dimension type identifier (e.g. "minecraft:overworld").
    pub dimension_type: String,
    /// The dimension name identifier (e.g. "minecraft:overworld").
    pub dimension_name: String,
    /// First 8 bytes of the SHA-256 hash of the world's seed.
    pub hashed_seed: i64,
    /// The player's game mode (0=Survival, 1=Creative, 2=Adventure, 3=Spectator).
    pub gamemode: u8,
    /// The previous game mode (-1 = undefined).
    pub previous_gamemode: i8,
    /// Whether the world is a debug mode world.
    pub is_debug: bool,
    /// Whether the world is a superflat world.
    pub is_flat: bool,
    /// Whether death location data is present.
    pub has_death_location: bool,
    /// The dimension name where the player died (only if has_death_location).
    pub death_dimension_name: String,
    /// The location where the player died (only if has_death_location).
    pub death_location: (i32, i32, i32),
    /// The number of ticks until the player can use a portal again.
    pub portal_cooldown: i32,
    /// Bitmask of data to keep (0x01 = attributes, 0x02 = metadata).
    pub data_kept: u8,
}

/// Serverbound Client Status packet (Play `0x08`).
///
/// Sent by the client to perform respawn after death or request stats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientStatus {
    /// The action ID (0 = Perform respawn, 1 = Request stats).
    pub action: i32,
}

/// Encodes a Set Health packet (clientbound Play `0x5B`).
pub fn encode_set_health(
    packet: &SetHealth,
    max_frame_length: usize,
    output: &mut Vec<u8>,
) -> Result<(), PlayError> {
    let mut body = Vec::new();
    encode_f32(packet.health, &mut body);
    encode_var_int(packet.food, &mut body);
    encode_f32(packet.food_saturation, &mut body);
    encode_frame(SET_HEALTH_PACKET_ID, &body, max_frame_length, output).map_err(PlayError::from)?;
    Ok(())
}

/// Decodes a Set Health packet (clientbound Play `0x5B`).
pub fn decode_set_health(
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
    if frame.packet_id != SET_HEALTH_PACKET_ID {
        *input = source;
        return Err(PlayError::WrongPacketId {
            received: frame.packet_id,
            expected: SET_HEALTH_PACKET_ID,
        });
    }
    let mut body = frame.payload;
    let health = decode_f32(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let food = decode_var_int(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let food_saturation = decode_f32(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    if !body.is_empty() {
        *input = source;
        return Err(PlayError::TrailingBytes { count: body.len() });
    }
    Ok(PlayDecodeOutcome::Complete(PlayPacket::SetHealth(
        SetHealth {
            health,
            food,
            food_saturation,
        },
    )))
}

/// Encodes a Respawn packet (clientbound Play `0x45`).
pub fn encode_respawn(
    packet: &Respawn,
    max_frame_length: usize,
    output: &mut Vec<u8>,
) -> Result<(), PlayError> {
    let mut body = Vec::new();
    encode_string(&packet.dimension_type, MAX_CHAT_COMPONENT_LENGTH, &mut body)
        .map_err(PlayError::from)?;
    encode_string(&packet.dimension_name, MAX_CHAT_COMPONENT_LENGTH, &mut body)
        .map_err(PlayError::from)?;
    encode_i64(packet.hashed_seed, &mut body);
    encode_u8(packet.gamemode, &mut body);
    encode_i8(packet.previous_gamemode, &mut body);
    encode_bool(packet.is_debug, &mut body);
    encode_bool(packet.is_flat, &mut body);
    encode_bool(packet.has_death_location, &mut body);
    if packet.has_death_location {
        encode_string(
            &packet.death_dimension_name,
            MAX_CHAT_COMPONENT_LENGTH,
            &mut body,
        )
        .map_err(PlayError::from)?;
        encode_position(
            packet.death_location.0,
            packet.death_location.1,
            packet.death_location.2,
            &mut body,
        );
    }
    encode_var_int(packet.portal_cooldown, &mut body);
    encode_u8(packet.data_kept, &mut body);
    encode_frame(RESPAWN_PACKET_ID, &body, max_frame_length, output).map_err(PlayError::from)?;
    Ok(())
}

/// Decodes a Respawn packet (clientbound Play `0x45`).
pub fn decode_respawn(
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
    if frame.packet_id != RESPAWN_PACKET_ID {
        *input = source;
        return Err(PlayError::WrongPacketId {
            received: frame.packet_id,
            expected: RESPAWN_PACKET_ID,
        });
    }
    let mut body = frame.payload;
    let dimension_type = decode_string(&mut body, MAX_CHAT_COMPONENT_LENGTH)
        .map_err(|e| {
            *input = source;
            PlayError::from(e)
        })?
        .to_string();
    let dimension_name = decode_string(&mut body, MAX_CHAT_COMPONENT_LENGTH)
        .map_err(|e| {
            *input = source;
            PlayError::from(e)
        })?
        .to_string();
    let hashed_seed = decode_i64(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let gamemode = decode_u8(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let previous_gamemode = decode_i8(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let is_debug = decode_bool(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let is_flat = decode_bool(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let has_death_location = decode_bool(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let (death_dimension_name, death_location) = if has_death_location {
        let dim = decode_string(&mut body, MAX_CHAT_COMPONENT_LENGTH)
            .map_err(|e| {
                *input = source;
                PlayError::from(e)
            })?
            .to_string();
        let pos = decode_position(&mut body).map_err(|e| {
            *input = source;
            PlayError::from(e)
        })?;
        (dim, pos)
    } else {
        (String::new(), (0, 0, 0))
    };
    let portal_cooldown = decode_var_int(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let data_kept = decode_u8(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    if !body.is_empty() {
        *input = source;
        return Err(PlayError::TrailingBytes { count: body.len() });
    }
    Ok(PlayDecodeOutcome::Complete(PlayPacket::Respawn(Respawn {
        dimension_type,
        dimension_name,
        hashed_seed,
        gamemode,
        previous_gamemode,
        is_debug,
        is_flat,
        has_death_location,
        death_dimension_name,
        death_location,
        portal_cooldown,
        data_kept,
    })))
}

/// Decodes a Client Status packet (serverbound Play `0x08`).
pub fn decode_client_status(
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
    if frame.packet_id != CLIENT_STATUS_PACKET_ID {
        *input = source;
        return Err(PlayError::WrongPacketId {
            received: frame.packet_id,
            expected: CLIENT_STATUS_PACKET_ID,
        });
    }
    let mut body = frame.payload;
    let action = decode_var_int(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    if !body.is_empty() {
        *input = source;
        return Err(PlayError::TrailingBytes { count: body.len() });
    }
    Ok(PlayDecodeOutcome::Complete(PlayPacket::ClientStatus(
        ClientStatus { action },
    )))
}

/// Encodes a slot to the given buffer.
///
/// Format: bool present, then (if present) VarInt item_id, i8 count, optional NBT.
pub fn encode_slot(slot: &Slot, output: &mut Vec<u8>) {
    encode_bool(slot.present, output);
    if slot.present {
        encode_var_int(slot.item_id, output);
        encode_i8(slot.count, output);
        if slot.nbt.is_empty() {
            // No NBT: write TAG_End
            output.push(0x00);
        } else {
            output.extend_from_slice(&slot.nbt);
        }
    }
}

/// Decodes a slot from the given input.
pub fn decode_slot(input: &mut &[u8]) -> Result<Slot, CodecError> {
    let present = decode_bool(input)?;
    if !present {
        return Ok(Slot::empty());
    }
    let item_id = decode_var_int(input)?;
    let count = decode_i8(input)?;
    // Read optional NBT: first byte determines if there's NBT
    if input.is_empty() {
        return Err(CodecError::IncompleteInput);
    }
    let first_byte = input[0];
    if first_byte == 0x00 {
        // TAG_End: no NBT
        *input = &input[1..];
        Ok(Slot {
            present: true,
            item_id,
            count,
            nbt: Vec::new(),
        })
    } else {
        // Read a full NBT compound
        let nbt = read_nbt_compound(input)?;
        Ok(Slot {
            present: true,
            item_id,
            count,
            nbt,
        })
    }
}

/// Reads a full NBT compound from the input, returning the raw bytes.
///
/// This is a minimal NBT reader that handles the TAG_Compound structure
/// by tracking nesting depth. It returns the raw bytes including the
/// initial TAG_Compound byte.
fn read_nbt_compound(input: &mut &[u8]) -> Result<Vec<u8>, CodecError> {
    let start = *input;
    if input.is_empty() {
        return Err(CodecError::IncompleteInput);
    }
    let tag_type = input[0];
    if tag_type != 0x0A {
        // Not a TAG_Compound - this is unexpected for slot NBT
        return Err(CodecError::InvalidBoolean);
    }

    // We need to parse the NBT to find its end.
    // A minimal approach: track depth and skip through the structure.
    let mut pos = 0;
    let nbt_bytes = parse_nbt_payload(start, &mut pos)?;
    *input = &start[pos..];
    Ok(nbt_bytes)
}

/// Parses an NBT payload starting at `pos`, returning the raw bytes consumed.
fn parse_nbt_payload(data: &[u8], pos: &mut usize) -> Result<Vec<u8>, CodecError> {
    let start = *pos;
    // Read the tag type
    if *pos >= data.len() {
        return Err(CodecError::IncompleteInput);
    }
    let tag_type = data[*pos];
    *pos += 1;

    match tag_type {
        0x00 => {
            // TAG_End
            Ok(data[start..*pos].to_vec())
        }
        0x01..=0x08 => {
            // Primitive types with fixed size: skip name + value
            skip_nbt_name(data, pos)?;
            skip_nbt_primitive(tag_type, data, pos)?;
            Ok(data[start..*pos].to_vec())
        }
        0x09 => {
            // TAG_List: skip name, then type byte, length, and elements
            skip_nbt_name(data, pos)?;
            if *pos + 5 > data.len() {
                return Err(CodecError::IncompleteInput);
            }
            let element_type = data[*pos];
            *pos += 1;
            let length =
                i32::from_be_bytes([data[*pos], data[*pos + 1], data[*pos + 2], data[*pos + 3]]);
            *pos += 4;
            for _ in 0..length {
                parse_nbt_list_element(element_type, data, pos)?;
            }
            Ok(data[start..*pos].to_vec())
        }
        0x0A => {
            // TAG_Compound: skip name, then read entries until TAG_End
            skip_nbt_name(data, pos)?;
            loop {
                if *pos >= data.len() {
                    return Err(CodecError::IncompleteInput);
                }
                let child_type = data[*pos];
                if child_type == 0x00 {
                    *pos += 1;
                    break;
                }
                // Read child entry: type + name + payload
                *pos += 1;
                skip_nbt_name(data, pos)?;
                parse_nbt_payload_inner(child_type, data, pos)?;
            }
            Ok(data[start..*pos].to_vec())
        }
        0x0B => {
            // TAG_Int_Array: skip name, length, and ints
            skip_nbt_name(data, pos)?;
            if *pos + 4 > data.len() {
                return Err(CodecError::IncompleteInput);
            }
            let length =
                i32::from_be_bytes([data[*pos], data[*pos + 1], data[*pos + 2], data[*pos + 3]]);
            *pos += 4;
            *pos += (length as usize) * 4;
            if *pos > data.len() {
                return Err(CodecError::IncompleteInput);
            }
            Ok(data[start..*pos].to_vec())
        }
        0x0C => {
            // TAG_Long_Array: skip name, length, and longs
            skip_nbt_name(data, pos)?;
            if *pos + 4 > data.len() {
                return Err(CodecError::IncompleteInput);
            }
            let length =
                i32::from_be_bytes([data[*pos], data[*pos + 1], data[*pos + 2], data[*pos + 3]]);
            *pos += 4;
            *pos += (length as usize) * 8;
            if *pos > data.len() {
                return Err(CodecError::IncompleteInput);
            }
            Ok(data[start..*pos].to_vec())
        }
        0x0D => {
            // TAG_Byte_Array: skip name, length, and bytes
            skip_nbt_name(data, pos)?;
            if *pos + 4 > data.len() {
                return Err(CodecError::IncompleteInput);
            }
            let length =
                i32::from_be_bytes([data[*pos], data[*pos + 1], data[*pos + 2], data[*pos + 3]]);
            *pos += 4;
            *pos += length as usize;
            if *pos > data.len() {
                return Err(CodecError::IncompleteInput);
            }
            Ok(data[start..*pos].to_vec())
        }
        _ => Err(CodecError::InvalidBoolean),
    }
}

/// Parses an NBT payload without a leading name (used inside lists and compound entries).
fn parse_nbt_payload_inner(tag_type: u8, data: &[u8], pos: &mut usize) -> Result<(), CodecError> {
    match tag_type {
        0x00 => Ok(()),
        0x01..=0x08 => {
            skip_nbt_primitive(tag_type, data, pos)?;
            Ok(())
        }
        0x09 => {
            if *pos + 5 > data.len() {
                return Err(CodecError::IncompleteInput);
            }
            let element_type = data[*pos];
            *pos += 1;
            let length =
                i32::from_be_bytes([data[*pos], data[*pos + 1], data[*pos + 2], data[*pos + 3]]);
            *pos += 4;
            for _ in 0..length {
                parse_nbt_list_element(element_type, data, pos)?;
            }
            Ok(())
        }
        0x0A => {
            loop {
                if *pos >= data.len() {
                    return Err(CodecError::IncompleteInput);
                }
                let child_type = data[*pos];
                if child_type == 0x00 {
                    *pos += 1;
                    break;
                }
                *pos += 1;
                skip_nbt_name(data, pos)?;
                parse_nbt_payload_inner(child_type, data, pos)?;
            }
            Ok(())
        }
        0x0B => {
            if *pos + 4 > data.len() {
                return Err(CodecError::IncompleteInput);
            }
            let length =
                i32::from_be_bytes([data[*pos], data[*pos + 1], data[*pos + 2], data[*pos + 3]]);
            *pos += 4;
            *pos += (length as usize) * 4;
            if *pos > data.len() {
                return Err(CodecError::IncompleteInput);
            }
            Ok(())
        }
        0x0C => {
            if *pos + 4 > data.len() {
                return Err(CodecError::IncompleteInput);
            }
            let length =
                i32::from_be_bytes([data[*pos], data[*pos + 1], data[*pos + 2], data[*pos + 3]]);
            *pos += 4;
            *pos += (length as usize) * 8;
            if *pos > data.len() {
                return Err(CodecError::IncompleteInput);
            }
            Ok(())
        }
        0x0D => {
            if *pos + 4 > data.len() {
                return Err(CodecError::IncompleteInput);
            }
            let length =
                i32::from_be_bytes([data[*pos], data[*pos + 1], data[*pos + 2], data[*pos + 3]]);
            *pos += 4;
            *pos += length as usize;
            if *pos > data.len() {
                return Err(CodecError::IncompleteInput);
            }
            Ok(())
        }
        _ => Err(CodecError::InvalidBoolean),
    }
}

/// Parses a single list element (no name prefix).
fn parse_nbt_list_element(
    element_type: u8,
    data: &[u8],
    pos: &mut usize,
) -> Result<(), CodecError> {
    match element_type {
        0x00 => Ok(()),
        0x01..=0x08 => {
            skip_nbt_primitive(element_type, data, pos)?;
            Ok(())
        }
        0x09 => {
            if *pos + 5 > data.len() {
                return Err(CodecError::IncompleteInput);
            }
            let inner_type = data[*pos];
            *pos += 1;
            let length =
                i32::from_be_bytes([data[*pos], data[*pos + 1], data[*pos + 2], data[*pos + 3]]);
            *pos += 4;
            for _ in 0..length {
                parse_nbt_list_element(inner_type, data, pos)?;
            }
            Ok(())
        }
        0x0A => {
            loop {
                if *pos >= data.len() {
                    return Err(CodecError::IncompleteInput);
                }
                let child_type = data[*pos];
                if child_type == 0x00 {
                    *pos += 1;
                    break;
                }
                *pos += 1;
                skip_nbt_name(data, pos)?;
                parse_nbt_payload_inner(child_type, data, pos)?;
            }
            Ok(())
        }
        0x0B => {
            if *pos + 4 > data.len() {
                return Err(CodecError::IncompleteInput);
            }
            let length =
                i32::from_be_bytes([data[*pos], data[*pos + 1], data[*pos + 2], data[*pos + 3]]);
            *pos += 4;
            *pos += (length as usize) * 4;
            if *pos > data.len() {
                return Err(CodecError::IncompleteInput);
            }
            Ok(())
        }
        0x0C => {
            if *pos + 4 > data.len() {
                return Err(CodecError::IncompleteInput);
            }
            let length =
                i32::from_be_bytes([data[*pos], data[*pos + 1], data[*pos + 2], data[*pos + 3]]);
            *pos += 4;
            *pos += (length as usize) * 8;
            if *pos > data.len() {
                return Err(CodecError::IncompleteInput);
            }
            Ok(())
        }
        0x0D => {
            if *pos + 4 > data.len() {
                return Err(CodecError::IncompleteInput);
            }
            let length =
                i32::from_be_bytes([data[*pos], data[*pos + 1], data[*pos + 2], data[*pos + 3]]);
            *pos += 4;
            *pos += length as usize;
            if *pos > data.len() {
                return Err(CodecError::IncompleteInput);
            }
            Ok(())
        }
        _ => Err(CodecError::InvalidBoolean),
    }
}

/// Skips an NBT name (2-byte length prefix + UTF-8 bytes).
fn skip_nbt_name(data: &[u8], pos: &mut usize) -> Result<(), CodecError> {
    if *pos + 2 > data.len() {
        return Err(CodecError::IncompleteInput);
    }
    let name_len = u16::from_be_bytes([data[*pos], data[*pos + 1]]) as usize;
    *pos += 2;
    *pos += name_len;
    if *pos > data.len() {
        return Err(CodecError::IncompleteInput);
    }
    Ok(())
}

/// Skips a primitive NBT value of the given type.
fn skip_nbt_primitive(tag_type: u8, data: &[u8], pos: &mut usize) -> Result<(), CodecError> {
    let size = match tag_type {
        0x01 => 1, // TAG_Byte
        0x02 => 2, // TAG_Short
        0x03 => 4, // TAG_Int
        0x04 => 8, // TAG_Long
        0x05 => 4, // TAG_Float
        0x06 => 8, // TAG_Double
        0x07 => 4, // TAG_Byte_Array (length prefix only; actual bytes handled by caller)
        0x08 => {
            // TAG_String: 2-byte length + bytes
            if *pos + 2 > data.len() {
                return Err(CodecError::IncompleteInput);
            }
            let len = u16::from_be_bytes([data[*pos], data[*pos + 1]]) as usize;
            *pos += 2 + len;
            if *pos > data.len() {
                return Err(CodecError::IncompleteInput);
            }
            return Ok(());
        }
        _ => return Err(CodecError::InvalidBoolean),
    };
    *pos += size;
    if *pos > data.len() {
        return Err(CodecError::IncompleteInput);
    }
    Ok(())
}

/// Encodes a Set Container Content packet (clientbound Play `0x12`).
pub fn encode_set_container_content(
    packet: &SetContainerContent,
    max_frame_length: usize,
    output: &mut Vec<u8>,
) -> Result<(), PlayError> {
    let mut body = Vec::new();
    encode_u8(packet.window_id, &mut body);
    encode_var_int(packet.state_id, &mut body);
    if packet.slots.len() > MAX_CONTAINER_SLOTS {
        return Err(PlayError::Codec(CodecError::VarIntTooLong));
    }
    encode_var_int(packet.slots.len() as i32, &mut body);
    for slot in &packet.slots {
        encode_slot(slot, &mut body);
    }
    encode_slot(&packet.carried_item, &mut body);
    encode_frame(
        SET_CONTAINER_CONTENT_PACKET_ID,
        &body,
        max_frame_length,
        output,
    )
    .map_err(PlayError::from)?;
    Ok(())
}

/// Decodes a Set Container Content packet (clientbound Play `0x12`).
pub fn decode_set_container_content(
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
    if frame.packet_id != SET_CONTAINER_CONTENT_PACKET_ID {
        *input = source;
        return Err(PlayError::WrongPacketId {
            received: frame.packet_id,
            expected: SET_CONTAINER_CONTENT_PACKET_ID,
        });
    }
    let mut body = frame.payload;
    let window_id = decode_u8(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let state_id = decode_var_int(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let slot_count = decode_var_int(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    if slot_count < 0 || slot_count as usize > MAX_CONTAINER_SLOTS {
        *input = source;
        return Err(PlayError::Codec(CodecError::VarIntTooLong));
    }
    let mut slots = Vec::with_capacity(slot_count as usize);
    for _ in 0..slot_count {
        slots.push(decode_slot(&mut body).map_err(|e| {
            *input = source;
            PlayError::from(e)
        })?);
    }
    let carried_item = decode_slot(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    if !body.is_empty() {
        *input = source;
        return Err(PlayError::TrailingBytes { count: body.len() });
    }
    Ok(PlayDecodeOutcome::Complete(
        PlayPacket::SetContainerContent(SetContainerContent {
            window_id,
            state_id,
            slots,
            carried_item,
        }),
    ))
}

/// Encodes a Set Container Slot packet (clientbound Play `0x14`).
pub fn encode_set_container_slot(
    packet: &SetContainerSlot,
    max_frame_length: usize,
    output: &mut Vec<u8>,
) -> Result<(), PlayError> {
    let mut body = Vec::new();
    encode_i8(packet.window_id, &mut body);
    encode_var_int(packet.state_id, &mut body);
    encode_i16(packet.slot, &mut body);
    encode_slot(&packet.item, &mut body);
    encode_frame(
        SET_CONTAINER_SLOT_PACKET_ID,
        &body,
        max_frame_length,
        output,
    )
    .map_err(PlayError::from)?;
    Ok(())
}

/// Decodes a Set Container Slot packet (clientbound Play `0x14`).
pub fn decode_set_container_slot(
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
    if frame.packet_id != SET_CONTAINER_SLOT_PACKET_ID {
        *input = source;
        return Err(PlayError::WrongPacketId {
            received: frame.packet_id,
            expected: SET_CONTAINER_SLOT_PACKET_ID,
        });
    }
    let mut body = frame.payload;
    let window_id = decode_i8(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let state_id = decode_var_int(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let slot = decode_i16(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let item = decode_slot(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    if !body.is_empty() {
        *input = source;
        return Err(PlayError::TrailingBytes { count: body.len() });
    }
    Ok(PlayDecodeOutcome::Complete(PlayPacket::SetContainerSlot(
        SetContainerSlot {
            window_id,
            state_id,
            slot,
            item,
        },
    )))
}

/// Decodes a Set Held Item serverbound packet (Play `0x28`).
pub fn decode_set_held_item_serverbound(
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
    if frame.packet_id != SET_HELD_ITEM_SERVERBOUND_PACKET_ID {
        *input = source;
        return Err(PlayError::WrongPacketId {
            received: frame.packet_id,
            expected: SET_HELD_ITEM_SERVERBOUND_PACKET_ID,
        });
    }
    let mut body = frame.payload;
    let slot = decode_i16(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    if !body.is_empty() {
        *input = source;
        return Err(PlayError::TrailingBytes { count: body.len() });
    }
    Ok(PlayDecodeOutcome::Complete(
        PlayPacket::SetHeldItemServerbound(SetHeldItemServerbound { slot }),
    ))
}

/// Decodes a Set Creative Mode Slot packet (serverbound Play `0x2B`).
pub fn decode_set_creative_mode_slot(
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
    if frame.packet_id != SET_CREATIVE_MODE_SLOT_PACKET_ID {
        *input = source;
        return Err(PlayError::WrongPacketId {
            received: frame.packet_id,
            expected: SET_CREATIVE_MODE_SLOT_PACKET_ID,
        });
    }
    let mut body = frame.payload;
    let slot = decode_i16(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let item = decode_slot(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    if !body.is_empty() {
        *input = source;
        return Err(PlayError::TrailingBytes { count: body.len() });
    }
    Ok(PlayDecodeOutcome::Complete(
        PlayPacket::SetCreativeModeSlot(SetCreativeModeSlot { slot, item }),
    ))
}

/// Serverbound Chat Message (Play `0x01`).
///
/// In offline mode, the signature fields are ignored. Only the `message`
/// field is used for chat broadcasting.
#[derive(Debug, Clone, PartialEq)]
pub struct ChatMessageServerbound {
    /// The chat message text (max 256 characters).
    pub message: String,
}

/// Clientbound System Chat Message (Play `0x5D`).
///
/// Used for system messages and, in offline mode, for player chat
/// (since signed chat is not applicable without online mode).
#[derive(Debug, Clone, PartialEq)]
pub struct SystemChatMessage {
    /// The chat content as a JSON chat component string.
    pub content: String,
    /// Whether this message should be shown as an overlay (above hotbar)
    /// rather than in the chat box.
    pub overlay: bool,
}

/// Encodes a Player Info Update packet (clientbound Play `0x3A`).
pub fn encode_player_info_update(
    packet: &PlayerInfoUpdate,
    max_frame_length: usize,
    output: &mut Vec<u8>,
) -> Result<(), PlayError> {
    let mut body = Vec::new();
    // Actions bitmask
    encode_u8(packet.actions.0, &mut body);
    // Number of entries
    encode_var_int(
        i32::try_from(packet.entries.len())
            .map_err(|_| PlayError::Codec(CodecError::VarIntTooLong))?,
        &mut body,
    );
    for entry in &packet.entries {
        // UUID
        body.extend_from_slice(&entry.uuid.to_be_bytes());
        // Add Player fields
        if packet.actions.has(PlayerInfoActions::ADD_PLAYER) {
            encode_string(&entry.name, MAX_CHAT_COMPONENT_LENGTH, &mut body)
                .map_err(PlayError::from)?;
            // Properties array
            encode_var_int(
                i32::try_from(entry.properties.len())
                    .map_err(|_| PlayError::Codec(CodecError::VarIntTooLong))?,
                &mut body,
            );
            for (name, value, signature) in &entry.properties {
                encode_string(name, MAX_CHAT_COMPONENT_LENGTH, &mut body)
                    .map_err(PlayError::from)?;
                encode_string(value, MAX_CHAT_COMPONENT_LENGTH, &mut body)
                    .map_err(PlayError::from)?;
                match signature {
                    Some(sig) => {
                        encode_bool(true, &mut body);
                        encode_string(sig, MAX_CHAT_COMPONENT_LENGTH, &mut body)
                            .map_err(PlayError::from)?;
                    }
                    None => encode_bool(false, &mut body),
                }
            }
        }
        // Update Gamemode
        if packet.actions.has(PlayerInfoActions::UPDATE_GAMEMODE) {
            encode_var_int(entry.gamemode, &mut body);
        }
        // Update Listed
        if packet.actions.has(PlayerInfoActions::UPDATE_LISTED) {
            encode_bool(entry.listed, &mut body);
        }
        // Update Latency
        if packet.actions.has(PlayerInfoActions::UPDATE_LATENCY) {
            encode_var_int(entry.latency, &mut body);
        }
        // Update Display Name
        if packet.actions.has(PlayerInfoActions::UPDATE_DISPLAY_NAME) {
            match &entry.display_name {
                Some(name) => {
                    encode_bool(true, &mut body);
                    encode_string(name, MAX_CHAT_COMPONENT_LENGTH, &mut body)
                        .map_err(PlayError::from)?;
                }
                None => encode_bool(false, &mut body),
            }
        }
    }
    encode_frame(
        PLAYER_INFO_UPDATE_PACKET_ID,
        &body,
        max_frame_length,
        output,
    )
    .map_err(PlayError::from)?;
    Ok(())
}

/// Encodes a Player Info Remove packet (clientbound Play `0x39`).
pub fn encode_player_info_remove(
    packet: &PlayerInfoRemove,
    max_frame_length: usize,
    output: &mut Vec<u8>,
) -> Result<(), PlayError> {
    let mut body = Vec::new();
    encode_var_int(
        i32::try_from(packet.uuids.len())
            .map_err(|_| PlayError::Codec(CodecError::VarIntTooLong))?,
        &mut body,
    );
    for uuid in &packet.uuids {
        body.extend_from_slice(&uuid.to_be_bytes());
    }
    encode_frame(
        PLAYER_INFO_REMOVE_PACKET_ID,
        &body,
        max_frame_length,
        output,
    )
    .map_err(PlayError::from)?;
    Ok(())
}

/// Encodes a Spawn Player packet (clientbound Play `0x03`).
pub fn encode_spawn_player(
    packet: &SpawnPlayer,
    max_frame_length: usize,
    output: &mut Vec<u8>,
) -> Result<(), PlayError> {
    let mut body = Vec::new();
    encode_var_int(packet.entity_id, &mut body);
    body.extend_from_slice(&packet.uuid.to_be_bytes());
    encode_f64(packet.x, &mut body);
    encode_f64(packet.y, &mut body);
    encode_f64(packet.z, &mut body);
    encode_u8(packet.yaw, &mut body);
    encode_u8(packet.pitch, &mut body);
    // Entity metadata is empty (terminated by 0xFF)
    body.push(0xFF);
    encode_frame(SPAWN_PLAYER_PACKET_ID, &body, max_frame_length, output)
        .map_err(PlayError::from)?;
    Ok(())
}

/// Encodes a Remove Entities packet (clientbound Play `0x3E`).
pub fn encode_remove_entities(
    packet: &RemoveEntities,
    max_frame_length: usize,
    output: &mut Vec<u8>,
) -> Result<(), PlayError> {
    let mut body = Vec::new();
    encode_var_int(
        i32::try_from(packet.entity_ids.len())
            .map_err(|_| PlayError::Codec(CodecError::VarIntTooLong))?,
        &mut body,
    );
    for id in &packet.entity_ids {
        encode_var_int(*id, &mut body);
    }
    encode_frame(REMOVE_ENTITIES_PACKET_ID, &body, max_frame_length, output)
        .map_err(PlayError::from)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Entity movement packets (protocol 763)
// ---------------------------------------------------------------------------

/// Encodes a Move Entity (Pos) packet (clientbound Play `0x29`).
pub fn encode_move_entity_pos(
    packet: &MoveEntityPos,
    max_frame_length: usize,
    output: &mut Vec<u8>,
) -> Result<(), PlayError> {
    let mut body = Vec::new();
    encode_var_int(packet.entity_id, &mut body);
    encode_i16(packet.delta_x, &mut body);
    encode_i16(packet.delta_y, &mut body);
    encode_i16(packet.delta_z, &mut body);
    encode_bool(packet.on_ground, &mut body);
    encode_frame(MOVE_ENTITY_POS_PACKET_ID, &body, max_frame_length, output)
        .map_err(PlayError::from)?;
    Ok(())
}

/// Decodes a Move Entity (Pos) packet (clientbound Play `0x29`).
pub fn decode_move_entity_pos(
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
    if frame.packet_id != MOVE_ENTITY_POS_PACKET_ID {
        *input = source;
        return Err(PlayError::WrongPacketId {
            received: frame.packet_id,
            expected: MOVE_ENTITY_POS_PACKET_ID,
        });
    }
    let mut body = frame.payload;
    let entity_id = decode_var_int(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let delta_x = decode_i16(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let delta_y = decode_i16(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let delta_z = decode_i16(&mut body).map_err(|e| {
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
    Ok(PlayDecodeOutcome::Complete(PlayPacket::MoveEntityPos(
        MoveEntityPos {
            entity_id,
            delta_x,
            delta_y,
            delta_z,
            on_ground,
        },
    )))
}

/// Encodes a Move Entity (Pos+Rot) packet (clientbound Play `0x2A`).
pub fn encode_move_entity_pos_rot(
    packet: &MoveEntityPosRot,
    max_frame_length: usize,
    output: &mut Vec<u8>,
) -> Result<(), PlayError> {
    let mut body = Vec::new();
    encode_var_int(packet.entity_id, &mut body);
    encode_i16(packet.delta_x, &mut body);
    encode_i16(packet.delta_y, &mut body);
    encode_i16(packet.delta_z, &mut body);
    encode_u8(packet.yaw, &mut body);
    encode_u8(packet.pitch, &mut body);
    encode_bool(packet.on_ground, &mut body);
    encode_frame(
        MOVE_ENTITY_POS_ROT_PACKET_ID,
        &body,
        max_frame_length,
        output,
    )
    .map_err(PlayError::from)?;
    Ok(())
}

/// Decodes a Move Entity (Pos+Rot) packet (clientbound Play `0x2A`).
pub fn decode_move_entity_pos_rot(
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
    if frame.packet_id != MOVE_ENTITY_POS_ROT_PACKET_ID {
        *input = source;
        return Err(PlayError::WrongPacketId {
            received: frame.packet_id,
            expected: MOVE_ENTITY_POS_ROT_PACKET_ID,
        });
    }
    let mut body = frame.payload;
    let entity_id = decode_var_int(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let delta_x = decode_i16(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let delta_y = decode_i16(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let delta_z = decode_i16(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let yaw = decode_u8(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let pitch = decode_u8(&mut body).map_err(|e| {
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
    Ok(PlayDecodeOutcome::Complete(PlayPacket::MoveEntityPosRot(
        MoveEntityPosRot {
            entity_id,
            delta_x,
            delta_y,
            delta_z,
            yaw,
            pitch,
            on_ground,
        },
    )))
}

/// Encodes a Move Entity (Rot) packet (clientbound Play `0x2B`).
pub fn encode_move_entity_rot(
    packet: &MoveEntityRot,
    max_frame_length: usize,
    output: &mut Vec<u8>,
) -> Result<(), PlayError> {
    let mut body = Vec::new();
    encode_var_int(packet.entity_id, &mut body);
    encode_u8(packet.yaw, &mut body);
    encode_u8(packet.pitch, &mut body);
    encode_bool(packet.on_ground, &mut body);
    encode_frame(MOVE_ENTITY_ROT_PACKET_ID, &body, max_frame_length, output)
        .map_err(PlayError::from)?;
    Ok(())
}

/// Decodes a Move Entity (Rot) packet (clientbound Play `0x2B`).
pub fn decode_move_entity_rot(
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
    if frame.packet_id != MOVE_ENTITY_ROT_PACKET_ID {
        *input = source;
        return Err(PlayError::WrongPacketId {
            received: frame.packet_id,
            expected: MOVE_ENTITY_ROT_PACKET_ID,
        });
    }
    let mut body = frame.payload;
    let entity_id = decode_var_int(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let yaw = decode_u8(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let pitch = decode_u8(&mut body).map_err(|e| {
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
    Ok(PlayDecodeOutcome::Complete(PlayPacket::MoveEntityRot(
        MoveEntityRot {
            entity_id,
            yaw,
            pitch,
            on_ground,
        },
    )))
}

/// Encodes an Entity Teleport packet (clientbound Play `0x57`).
pub fn encode_entity_teleport(
    packet: &EntityTeleport,
    max_frame_length: usize,
    output: &mut Vec<u8>,
) -> Result<(), PlayError> {
    let mut body = Vec::new();
    encode_var_int(packet.entity_id, &mut body);
    encode_f64(packet.x, &mut body);
    encode_f64(packet.y, &mut body);
    encode_f64(packet.z, &mut body);
    encode_u8(packet.yaw, &mut body);
    encode_u8(packet.pitch, &mut body);
    encode_bool(packet.on_ground, &mut body);
    encode_frame(ENTITY_TELEPORT_PACKET_ID, &body, max_frame_length, output)
        .map_err(PlayError::from)?;
    Ok(())
}

/// Decodes an Entity Teleport packet (clientbound Play `0x57`).
pub fn decode_entity_teleport(
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
    if frame.packet_id != ENTITY_TELEPORT_PACKET_ID {
        *input = source;
        return Err(PlayError::WrongPacketId {
            received: frame.packet_id,
            expected: ENTITY_TELEPORT_PACKET_ID,
        });
    }
    let mut body = frame.payload;
    let entity_id = decode_var_int(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let x = decode_f64(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let y = decode_f64(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let z = decode_f64(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let yaw = decode_u8(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let pitch = decode_u8(&mut body).map_err(|e| {
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
    Ok(PlayDecodeOutcome::Complete(PlayPacket::EntityTeleport(
        EntityTeleport {
            entity_id,
            x,
            y,
            z,
            yaw,
            pitch,
            on_ground,
        },
    )))
}

// ---------------------------------------------------------------------------
// Chat packets (protocol 763)
// ---------------------------------------------------------------------------

/// Encodes a System Chat Message packet (clientbound Play `0x5D`).
pub fn encode_system_chat_message(
    packet: &SystemChatMessage,
    max_frame_length: usize,
    output: &mut Vec<u8>,
) -> Result<(), PlayError> {
    let mut body = Vec::new();
    encode_string(&packet.content, 32767, &mut body).map_err(PlayError::Codec)?;
    encode_bool(packet.overlay, &mut body);
    encode_frame(
        SYSTEM_CHAT_MESSAGE_PACKET_ID,
        &body,
        max_frame_length,
        output,
    )
    .map_err(PlayError::from)?;
    Ok(())
}

/// Decodes a Chat Message packet (serverbound Play `0x01`).
///
/// In offline mode, only the `message` field is extracted. The remaining
/// fields (timestamp, salt, signature, acknowledgements) are consumed
/// and discarded, as they are only relevant for signed chat (online mode).
pub fn decode_chat_message_serverbound(
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
    if frame.packet_id != CHAT_MESSAGE_SERVERBOUND_PACKET_ID {
        *input = source;
        return Err(PlayError::WrongPacketId {
            received: frame.packet_id,
            expected: CHAT_MESSAGE_SERVERBOUND_PACKET_ID,
        });
    }
    let mut body = frame.payload;
    // Message: String (max 256)
    let message = decode_string(&mut body, 256).map_err(|e| {
        *input = source;
        PlayError::Codec(e)
    })?;
    let message = message.to_string();
    // Remaining fields (timestamp, salt, signature, acknowledgements) are
    // skipped for offline mode. We don't need to parse them individually;
    // the frame boundary ensures we consume the right number of bytes.
    Ok(PlayDecodeOutcome::Complete(
        PlayPacket::ChatMessageServerbound(ChatMessageServerbound { message }),
    ))
}

// ---------------------------------------------------------------------------
// Block interaction packets (protocol 763)
// ---------------------------------------------------------------------------

/// Player digging action type (serverbound Play `0x1D`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerDiggingAction {
    /// Start destroying block.
    StartDestroy = 0,
    /// Abort destroying block.
    AbortDestroy = 1,
    /// Stop destroying block.
    StopDestroy = 2,
    /// Drop all items (ctrl+drop).
    DropAllItems = 3,
    /// Drop item (q).
    DropItem = 4,
    /// Shoot arrow / finish eating.
    ShootArrowOrFinishEating = 5,
    /// Swap item in hand.
    SwapItemInHand = 6,
}

impl PlayerDiggingAction {
    /// Converts from wire value.
    pub fn from_wire(value: i32) -> Option<Self> {
        match value {
            0 => Some(Self::StartDestroy),
            1 => Some(Self::AbortDestroy),
            2 => Some(Self::StopDestroy),
            3 => Some(Self::DropAllItems),
            4 => Some(Self::DropItem),
            5 => Some(Self::ShootArrowOrFinishEating),
            6 => Some(Self::SwapItemInHand),
            _ => None,
        }
    }
}

/// Serverbound Player Digging packet (Play `0x1D`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayerDigging {
    /// The digging action.
    pub action: PlayerDiggingAction,
    /// The block position.
    pub position: (i32, i32, i32),
    /// The face being dug (0-5).
    pub face: u8,
    /// The sequence number for Acknowledge Block Change.
    pub sequence: i32,
}

/// Serverbound Use Item On (Place Block) packet (Play `0x31`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UseItemOn {
    /// The block position being targeted.
    pub position: (i32, i32, i32),
    /// The face being targeted (0-5).
    pub face: u8,
    /// The hand used (0=main, 1=off).
    pub hand: i32,
    /// The cursor X position (0-1024, where 0 is 0 and 1024 is 1.0).
    pub cursor_x: f32,
    /// The cursor Y position.
    pub cursor_y: f32,
    /// The cursor Z position.
    pub cursor_z: f32,
    /// Whether the player's head is inside a block.
    pub inside_block: bool,
    /// The sequence number for Acknowledge Block Change.
    pub sequence: i32,
}

/// Clientbound Block Update packet (Play `0x0A`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockUpdate {
    /// The block position.
    pub position: (i32, i32, i32),
    /// The block state ID.
    pub block_state: i32,
}

/// Clientbound Acknowledge Block Change packet (Play `0x06`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AcknowledgeBlockChange {
    /// The sequence number to acknowledge.
    pub sequence: i32,
}

/// Encodes a Player Digging packet (serverbound Play `0x1D`).
pub fn encode_player_digging(
    packet: &PlayerDigging,
    max_frame_length: usize,
    output: &mut Vec<u8>,
) -> Result<(), PlayError> {
    let mut body = Vec::new();
    encode_var_int(packet.action as i32, &mut body);
    encode_position(
        packet.position.0,
        packet.position.1,
        packet.position.2,
        &mut body,
    );
    encode_u8(packet.face, &mut body);
    encode_var_int(packet.sequence, &mut body);
    encode_frame(PLAYER_DIGGING_PACKET_ID, &body, max_frame_length, output)
        .map_err(PlayError::from)?;
    Ok(())
}

/// Decodes a Player Digging packet (serverbound Play `0x1D`).
pub fn decode_player_digging(
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
    if frame.packet_id != PLAYER_DIGGING_PACKET_ID {
        *input = source;
        return Err(PlayError::WrongPacketId {
            received: frame.packet_id,
            expected: PLAYER_DIGGING_PACKET_ID,
        });
    }
    let mut body = frame.payload;
    let action_raw = decode_var_int(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let action = PlayerDiggingAction::from_wire(action_raw).ok_or_else(|| {
        *input = source;
        PlayError::Codec(CodecError::InvalidBoolean)
    })?;
    let (x, y, z) = decode_position(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let face = decode_u8(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let sequence = decode_var_int(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    if !body.is_empty() {
        *input = source;
        return Err(PlayError::TrailingBytes { count: body.len() });
    }
    Ok(PlayDecodeOutcome::Complete(PlayPacket::PlayerDigging(
        PlayerDigging {
            action,
            position: (x, y, z),
            face,
            sequence,
        },
    )))
}

/// Encodes a Use Item On packet (serverbound Play `0x31`).
pub fn encode_use_item_on(
    packet: &UseItemOn,
    max_frame_length: usize,
    output: &mut Vec<u8>,
) -> Result<(), PlayError> {
    let mut body = Vec::new();
    encode_position(
        packet.position.0,
        packet.position.1,
        packet.position.2,
        &mut body,
    );
    encode_u8(packet.face, &mut body);
    encode_var_int(packet.hand, &mut body);
    encode_f32(packet.cursor_x, &mut body);
    encode_f32(packet.cursor_y, &mut body);
    encode_f32(packet.cursor_z, &mut body);
    encode_bool(packet.inside_block, &mut body);
    encode_var_int(packet.sequence, &mut body);
    encode_frame(USE_ITEM_ON_PACKET_ID, &body, max_frame_length, output)
        .map_err(PlayError::from)?;
    Ok(())
}

/// Decodes a Use Item On packet (serverbound Play `0x31`).
pub fn decode_use_item_on(
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
    if frame.packet_id != USE_ITEM_ON_PACKET_ID {
        *input = source;
        return Err(PlayError::WrongPacketId {
            received: frame.packet_id,
            expected: USE_ITEM_ON_PACKET_ID,
        });
    }
    let mut body = frame.payload;
    let (x, y, z) = decode_position(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let face = decode_u8(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let hand = decode_var_int(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let cursor_x = decode_f32(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let cursor_y = decode_f32(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let cursor_z = decode_f32(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let inside_block = decode_bool(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let sequence = decode_var_int(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    if !body.is_empty() {
        *input = source;
        return Err(PlayError::TrailingBytes { count: body.len() });
    }
    Ok(PlayDecodeOutcome::Complete(PlayPacket::UseItemOn(
        UseItemOn {
            position: (x, y, z),
            face,
            hand,
            cursor_x,
            cursor_y,
            cursor_z,
            inside_block,
            sequence,
        },
    )))
}

/// Encodes a Block Update packet (clientbound Play `0x0A`).
pub fn encode_block_update(
    packet: &BlockUpdate,
    max_frame_length: usize,
    output: &mut Vec<u8>,
) -> Result<(), PlayError> {
    let mut body = Vec::new();
    encode_position(
        packet.position.0,
        packet.position.1,
        packet.position.2,
        &mut body,
    );
    encode_var_int(packet.block_state, &mut body);
    encode_frame(BLOCK_UPDATE_PACKET_ID, &body, max_frame_length, output)
        .map_err(PlayError::from)?;
    Ok(())
}

/// Decodes a Block Update packet (clientbound Play `0x0A`).
pub fn decode_block_update(
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
    if frame.packet_id != BLOCK_UPDATE_PACKET_ID {
        *input = source;
        return Err(PlayError::WrongPacketId {
            received: frame.packet_id,
            expected: BLOCK_UPDATE_PACKET_ID,
        });
    }
    let mut body = frame.payload;
    let (x, y, z) = decode_position(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    let block_state = decode_var_int(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    if !body.is_empty() {
        *input = source;
        return Err(PlayError::TrailingBytes { count: body.len() });
    }
    Ok(PlayDecodeOutcome::Complete(PlayPacket::BlockUpdate(
        BlockUpdate {
            position: (x, y, z),
            block_state,
        },
    )))
}

/// Encodes an Acknowledge Block Change packet (clientbound Play `0x06`).
pub fn encode_acknowledge_block_change(
    packet: &AcknowledgeBlockChange,
    max_frame_length: usize,
    output: &mut Vec<u8>,
) -> Result<(), PlayError> {
    let mut body = Vec::new();
    encode_var_int(packet.sequence, &mut body);
    encode_frame(
        ACKNOWLEDGE_BLOCK_CHANGE_PACKET_ID,
        &body,
        max_frame_length,
        output,
    )
    .map_err(PlayError::from)?;
    Ok(())
}

/// Decodes an Acknowledge Block Change packet (clientbound Play `0x06`).
pub fn decode_acknowledge_block_change(
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
    if frame.packet_id != ACKNOWLEDGE_BLOCK_CHANGE_PACKET_ID {
        *input = source;
        return Err(PlayError::WrongPacketId {
            received: frame.packet_id,
            expected: ACKNOWLEDGE_BLOCK_CHANGE_PACKET_ID,
        });
    }
    let mut body = frame.payload;
    let sequence = decode_var_int(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    if !body.is_empty() {
        *input = source;
        return Err(PlayError::TrailingBytes { count: body.len() });
    }
    Ok(PlayDecodeOutcome::Complete(
        PlayPacket::AcknowledgeBlockChange(AcknowledgeBlockChange { sequence }),
    ))
}

// ---------------------------------------------------------------------------
// Chunk Data and Update Light (clientbound Play 0x24)
// ---------------------------------------------------------------------------

/// Encodes a Chunk Data and Update Light packet (clientbound Play `0x24`).
///
/// Wire layout for protocol 763 (1.20.1):
/// - Heightmaps: raw NBT (self-delimiting; not length-prefixed)
/// - Data: Prefixed Array of Byte (section buffer)
/// - Block entities: Prefixed Array (opaque entries for now)
/// - Light data: inline (BitSets + Prefixed Arrays); not wrapped in a byte array
pub fn encode_chunk_data(
    packet: &ChunkData,
    max_frame_length: usize,
    output: &mut Vec<u8>,
) -> Result<(), PlayError> {
    let mut body = Vec::new();
    encode_i32(packet.chunk_x, &mut body);
    encode_i32(packet.chunk_z, &mut body);

    // Heightmaps are raw NBT on the wire (self-delimiting; NOT length-prefixed).
    if packet.heightmaps.len() > MAX_CHUNK_DATA_SIZE {
        return Err(PlayError::Codec(CodecError::StringTooLong));
    }
    body.extend_from_slice(&packet.heightmaps);

    // Chunk sections and biomes (length-prefixed buffer)
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

    // Light data is inlined after block entities (NOT length-prefixed).
    if packet.light_data.len() > MAX_CHUNK_DATA_SIZE {
        return Err(PlayError::Codec(CodecError::StringTooLong));
    }
    body.extend_from_slice(&packet.light_data);

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
    // Heightmaps are raw NBT (self-delimiting).
    let heightmaps = read_nbt_compound(&mut body).map_err(|e| {
        *input = source;
        PlayError::from(e)
    })?;
    if heightmaps.len() > MAX_CHUNK_DATA_SIZE {
        *input = source;
        return Err(PlayError::Codec(CodecError::StringTooLong));
    }
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

    // Light data consumes the remainder of the packet body (inline, not length-prefixed).
    if body.len() > MAX_CHUNK_DATA_SIZE {
        *input = source;
        return Err(PlayError::Codec(CodecError::StringTooLong));
    }
    let light_data = body.to_vec();

    Ok(PlayDecodeOutcome::Complete(PlayPacket::ChunkData(
        ChunkData {
            chunk_x,
            chunk_z,
            heightmaps,
            data: data.to_vec(),
            block_entities,
            light_data,
        },
    )))
}

#[cfg(test)]
mod tests {
    use super::{
        AcknowledgeBlockChange, BlockUpdate, CHAT_MESSAGE_SERVERBOUND_PACKET_ID,
        CLIENT_STATUS_PACKET_ID, ChangeDifficulty, ChatMode, ChunkData, ClientInformation,
        CombatDeath, ConfirmTeleportation, DisconnectPlay, EntityEvent, EntityTeleport, GameEvent,
        GameMode, JOIN_GAME_PACKET_ID, JoinGame, KEEP_ALIVE_CLIENTBOUND_PACKET_ID, KeepAlive,
        MainHand, MoveEntityPos, MoveEntityPosRot, MoveEntityRot, PlayDecodeOutcome, PlayError,
        PlayPacket, PlayerAbilities, PlayerDigging, PlayerDiggingAction, PlayerInfoActions,
        PlayerInfoEntry, PlayerInfoRemove, PlayerInfoUpdate, PluginMessageClientbound,
        RemoveEntities, Respawn, SET_CREATIVE_MODE_SLOT_PACKET_ID,
        SET_HELD_ITEM_SERVERBOUND_PACKET_ID, SYSTEM_CHAT_MESSAGE_PACKET_ID, SetBlockDestroyStage,
        SetCenterChunk, SetContainerContent, SetContainerSlot, SetDefaultSpawnPosition, SetHealth,
        SetHeldItem, SetPlayerPosition, SetPlayerPositionAndRotation, SetPlayerRotation,
        SetRenderDistance, SetSimulationDistance, Slot, SpawnPlayer, SynchronizePlayerPosition,
        SystemChatMessage, UPDATE_RECIPES_PACKET_ID, UPDATE_TAGS_PACKET_ID, UnloadChunk, UseItemOn,
        decode_acknowledge_block_change, decode_block_update, decode_change_difficulty,
        decode_chat_message_serverbound, decode_chunk_data, decode_client_information,
        decode_client_status, decode_combat_death, decode_confirm_teleportation,
        decode_disconnect_play, decode_entity_event, decode_entity_teleport, decode_game_event,
        decode_join_game, decode_keep_alive_clientbound, decode_keep_alive_serverbound,
        decode_move_entity_pos, decode_move_entity_pos_rot, decode_move_entity_rot,
        decode_player_abilities, decode_player_digging, decode_plugin_message_clientbound,
        decode_respawn, decode_set_block_destroy_stage, decode_set_center_chunk,
        decode_set_container_content, decode_set_container_slot, decode_set_creative_mode_slot,
        decode_set_default_spawn_position, decode_set_health, decode_set_held_item,
        decode_set_held_item_serverbound, decode_set_player_position,
        decode_set_player_position_and_rotation, decode_set_player_rotation,
        decode_set_render_distance, decode_set_simulation_distance, decode_slot,
        decode_synchronize_player_position, decode_unload_chunk, decode_use_item_on,
        encode_acknowledge_block_change, encode_block_update, encode_change_difficulty,
        encode_chunk_data, encode_client_information, encode_combat_death,
        encode_confirm_teleportation, encode_disconnect_play, encode_entity_event,
        encode_entity_teleport, encode_game_event, encode_join_game, encode_keep_alive_clientbound,
        encode_keep_alive_serverbound, encode_move_entity_pos, encode_move_entity_pos_rot,
        encode_move_entity_rot, encode_player_abilities, encode_player_digging,
        encode_player_info_remove, encode_player_info_update, encode_plugin_message_clientbound,
        encode_remove_entities, encode_respawn, encode_set_block_destroy_stage,
        encode_set_center_chunk, encode_set_container_content, encode_set_container_slot,
        encode_set_default_spawn_position, encode_set_health, encode_set_held_item,
        encode_set_player_position, encode_set_player_position_and_rotation,
        encode_set_player_rotation, encode_set_render_distance, encode_set_simulation_distance,
        encode_slot, encode_spawn_player, encode_synchronize_player_position,
        encode_system_chat_message, encode_unload_chunk, encode_update_recipes_empty,
        encode_update_tags_empty, encode_use_item_on, ensure_play_state,
    };
    use crate::primitives::{decode_bool, decode_string};
    use crate::state::ProtocolState;

    const TEST_MAX_FRAME: usize = 1048576;

    fn sample_join_game() -> JoinGame {
        JoinGame {
            entity_id: 42,
            is_hardcore: false,
            gamemode: GameMode::Survival,
            previous_gamemode: None,
            dimension_names: vec!["minecraft:overworld".to_string()],
            registry_codec: crate::registry_codec::empty_nbt_compound(),
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
            has_death_location: false,
            death_dimension_name: String::new(),
            death_location: (0, 0, 0),
            portal_cooldown: 0,
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
        body.extend_from_slice(&crate::registry_codec::empty_nbt_compound());
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
        crate::primitives::encode_bool(false, &mut body);
        crate::primitives::encode_var_int(0, &mut body);

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
        body.extend_from_slice(&packet.registry_codec);
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
        crate::primitives::encode_bool(false, &mut body);
        crate::primitives::encode_var_int(0, &mut body);
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
            heightmaps: crate::registry_codec::empty_nbt_compound(),
            data: Vec::new(),
            block_entities: Vec::new(),
            light_data: Vec::new(),
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
            heightmaps: crate::registry_codec::empty_nbt_compound(),
            data: vec![0x01, 0x02, 0x03, 0x04],
            block_entities: vec![vec![0x0a, 0x01], vec![0x0a, 0x02]],
            light_data: vec![0x00], // BitSet length 0 placeholder byte for smoke
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
        body.extend_from_slice(&crate::registry_codec::empty_nbt_compound());
        crate::primitives::encode_byte_array(&[], 1048576, &mut body).map_err(PlayError::from)?;
        crate::primitives::encode_var_int(0, &mut body);
        // empty light data (remainder)

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
            heightmaps: crate::registry_codec::empty_nbt_compound(),
            data: vec![0x01],
            block_entities: Vec::new(),
            light_data: Vec::new(),
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

    // --- Confirm Teleportation ---

    #[test]
    fn confirm_teleportation_roundtrip() -> Result<(), PlayError> {
        let packet = ConfirmTeleportation { teleport_id: 42 };
        let mut wire = Vec::new();
        encode_confirm_teleportation(&packet, TEST_MAX_FRAME, &mut wire)?;
        let mut input = wire.as_slice();
        let decoded = decode_confirm_teleportation(&mut input, TEST_MAX_FRAME)?;
        match decoded {
            PlayDecodeOutcome::Complete(PlayPacket::ConfirmTeleportation(ct)) => {
                assert_eq!(ct.teleport_id, 42);
            }
            other => panic!("expected ConfirmTeleportation, got {other:?}"),
        }
        assert!(input.is_empty());
        Ok(())
    }

    #[test]
    fn confirm_teleportation_zero_teleport_id() -> Result<(), PlayError> {
        let packet = ConfirmTeleportation { teleport_id: 0 };
        let mut wire = Vec::new();
        encode_confirm_teleportation(&packet, TEST_MAX_FRAME, &mut wire)?;
        let mut input = wire.as_slice();
        let decoded = decode_confirm_teleportation(&mut input, TEST_MAX_FRAME)?;
        match decoded {
            PlayDecodeOutcome::Complete(PlayPacket::ConfirmTeleportation(ct)) => {
                assert_eq!(ct.teleport_id, 0);
            }
            other => panic!("expected ConfirmTeleportation, got {other:?}"),
        }
        Ok(())
    }

    #[test]
    fn confirm_teleportation_large_teleport_id() -> Result<(), PlayError> {
        let packet = ConfirmTeleportation {
            teleport_id: 2_147_483_647,
        };
        let mut wire = Vec::new();
        encode_confirm_teleportation(&packet, TEST_MAX_FRAME, &mut wire)?;
        let mut input = wire.as_slice();
        let decoded = decode_confirm_teleportation(&mut input, TEST_MAX_FRAME)?;
        match decoded {
            PlayDecodeOutcome::Complete(PlayPacket::ConfirmTeleportation(ct)) => {
                assert_eq!(ct.teleport_id, 2_147_483_647);
            }
            other => panic!("expected ConfirmTeleportation, got {other:?}"),
        }
        Ok(())
    }

    #[test]
    fn confirm_teleportation_truncated_is_incomplete() -> Result<(), PlayError> {
        let packet = ConfirmTeleportation { teleport_id: 100 };
        let mut wire = Vec::new();
        encode_confirm_teleportation(&packet, TEST_MAX_FRAME, &mut wire)?;

        for split in 0..wire.len() {
            let mut input = &wire[..split];
            assert_eq!(
                decode_confirm_teleportation(&mut input, TEST_MAX_FRAME)?,
                PlayDecodeOutcome::Incomplete
            );
        }
        Ok(())
    }

    #[test]
    fn confirm_teleportation_wrong_packet_id() -> Result<(), PlayError> {
        // Build a frame with the wrong packet ID (0x01 instead of 0x00)
        let mut body = Vec::new();
        crate::primitives::encode_var_int(5, &mut body);
        crate::framing::encode_frame(0x01, &body, TEST_MAX_FRAME, &mut Vec::new())
            .map_err(PlayError::from)?;
        let mut wire = Vec::new();
        crate::framing::encode_frame(0x01, &body, TEST_MAX_FRAME, &mut wire)
            .map_err(PlayError::from)?;
        let mut input = wire.as_slice();
        let result = decode_confirm_teleportation(&mut input, TEST_MAX_FRAME);
        assert!(matches!(result, Err(PlayError::WrongPacketId { .. })));
        Ok(())
    }

    #[test]
    fn confirm_teleportation_trailing_bytes() -> Result<(), PlayError> {
        let mut body = Vec::new();
        crate::primitives::encode_var_int(7, &mut body);
        body.push(0xff); // trailing byte
        let mut wire = Vec::new();
        crate::framing::encode_frame(
            super::CONFIRM_TELEPORTATION_PACKET_ID,
            &body,
            TEST_MAX_FRAME,
            &mut wire,
        )
        .map_err(PlayError::from)?;
        let mut input = wire.as_slice();
        let result = decode_confirm_teleportation(&mut input, TEST_MAX_FRAME);
        assert!(matches!(result, Err(PlayError::TrailingBytes { .. })));
        Ok(())
    }

    // --- Client Information ---

    #[test]
    fn client_information_roundtrip() -> Result<(), PlayError> {
        let packet = ClientInformation {
            locale: "en_US".to_string(),
            view_distance: 12,
            chat_mode: ChatMode::Full,
            chat_colors: true,
            displayed_skin_parts: 0x7f,
            main_hand: MainHand::Right,
            enable_text_filtering: false,
            allow_server_listings: true,
        };
        let mut wire = Vec::new();
        encode_client_information(&packet, TEST_MAX_FRAME, &mut wire)?;
        let mut input = wire.as_slice();
        let decoded = decode_client_information(&mut input, TEST_MAX_FRAME)?;
        match decoded {
            PlayDecodeOutcome::Complete(PlayPacket::ClientInformation(ci)) => {
                assert_eq!(ci.locale, "en_US");
                assert_eq!(ci.view_distance, 12);
                assert_eq!(ci.chat_mode, ChatMode::Full);
                assert!(ci.chat_colors);
                assert_eq!(ci.displayed_skin_parts, 0x7f);
                assert_eq!(ci.main_hand, MainHand::Right);
                assert!(!ci.enable_text_filtering);
                assert!(ci.allow_server_listings);
            }
            other => panic!("expected ClientInformation, got {other:?}"),
        }
        assert!(input.is_empty());
        Ok(())
    }

    #[test]
    fn client_information_all_chat_modes() -> Result<(), PlayError> {
        for mode in [ChatMode::Full, ChatMode::System, ChatMode::Hidden] {
            let packet = ClientInformation {
                locale: "ja_JP".to_string(),
                view_distance: 8,
                chat_mode: mode,
                chat_colors: false,
                displayed_skin_parts: 0,
                main_hand: MainHand::Left,
                enable_text_filtering: true,
                allow_server_listings: false,
            };
            let mut wire = Vec::new();
            encode_client_information(&packet, TEST_MAX_FRAME, &mut wire)?;
            let mut input = wire.as_slice();
            let decoded = decode_client_information(&mut input, TEST_MAX_FRAME)?;
            match decoded {
                PlayDecodeOutcome::Complete(PlayPacket::ClientInformation(ci)) => {
                    assert_eq!(ci.chat_mode, mode);
                    assert_eq!(ci.main_hand, MainHand::Left);
                }
                other => panic!("expected ClientInformation, got {other:?}"),
            }
        }
        Ok(())
    }

    #[test]
    fn client_information_truncated_is_incomplete() -> Result<(), PlayError> {
        let packet = ClientInformation {
            locale: "en_US".to_string(),
            view_distance: 10,
            chat_mode: ChatMode::Full,
            chat_colors: true,
            displayed_skin_parts: 0x7f,
            main_hand: MainHand::Right,
            enable_text_filtering: false,
            allow_server_listings: true,
        };
        let mut wire = Vec::new();
        encode_client_information(&packet, TEST_MAX_FRAME, &mut wire)?;

        for split in 0..wire.len() {
            let mut input = &wire[..split];
            assert_eq!(
                decode_client_information(&mut input, TEST_MAX_FRAME)?,
                PlayDecodeOutcome::Incomplete
            );
        }
        Ok(())
    }

    #[test]
    fn client_information_wrong_packet_id() -> Result<(), PlayError> {
        let mut body = Vec::new();
        crate::primitives::encode_string("en_US", 32767, &mut body).map_err(PlayError::from)?;
        crate::primitives::encode_u8(10, &mut body);
        crate::primitives::encode_var_int(0, &mut body);
        crate::primitives::encode_bool(true, &mut body);
        crate::primitives::encode_u8(0x7f, &mut body);
        crate::primitives::encode_var_int(1, &mut body);
        crate::primitives::encode_bool(false, &mut body);
        crate::primitives::encode_bool(true, &mut body);
        let mut wire = Vec::new();
        crate::framing::encode_frame(0x09, &body, TEST_MAX_FRAME, &mut wire)
            .map_err(PlayError::from)?;
        let mut input = wire.as_slice();
        let result = decode_client_information(&mut input, TEST_MAX_FRAME);
        assert!(matches!(result, Err(PlayError::WrongPacketId { .. })));
        Ok(())
    }

    // --- Join sequence packets ---

    #[test]
    fn plugin_message_brand_roundtrip() -> Result<(), PlayError> {
        let packet = PluginMessageClientbound {
            channel: "minecraft:brand".to_string(),
            data: b"rustbound".to_vec(),
        };
        let mut wire = Vec::new();
        encode_plugin_message_clientbound(&packet, TEST_MAX_FRAME, &mut wire)?;
        let mut input = wire.as_slice();
        match decode_plugin_message_clientbound(&mut input, TEST_MAX_FRAME)? {
            PlayDecodeOutcome::Complete(PlayPacket::PluginMessageClientbound(pm)) => {
                assert_eq!(pm.channel, "minecraft:brand");
                assert_eq!(pm.data, b"rustbound");
            }
            other => panic!("expected PluginMessageClientbound, got {other:?}"),
        }
        assert!(input.is_empty());
        Ok(())
    }

    #[test]
    fn change_difficulty_roundtrip() -> Result<(), PlayError> {
        let packet = ChangeDifficulty {
            difficulty: 2,
            locked: true,
        };
        let mut wire = Vec::new();
        encode_change_difficulty(&packet, TEST_MAX_FRAME, &mut wire)?;
        let mut input = wire.as_slice();
        match decode_change_difficulty(&mut input, TEST_MAX_FRAME)? {
            PlayDecodeOutcome::Complete(PlayPacket::ChangeDifficulty(cd)) => {
                assert_eq!(cd.difficulty, 2);
                assert!(cd.locked);
            }
            other => panic!("expected ChangeDifficulty, got {other:?}"),
        }
        assert!(input.is_empty());
        Ok(())
    }

    #[test]
    fn player_abilities_roundtrip() -> Result<(), PlayError> {
        let packet = PlayerAbilities {
            flags: 0x05,
            flying_speed: 0.05,
            fov_modifier: 0.1,
        };
        let mut wire = Vec::new();
        encode_player_abilities(&packet, TEST_MAX_FRAME, &mut wire)?;
        let mut input = wire.as_slice();
        match decode_player_abilities(&mut input, TEST_MAX_FRAME)? {
            PlayDecodeOutcome::Complete(PlayPacket::PlayerAbilities(pa)) => {
                assert_eq!(pa.flags, 0x05);
                assert_eq!(pa.flying_speed, 0.05);
                assert_eq!(pa.fov_modifier, 0.1);
            }
            other => panic!("expected PlayerAbilities, got {other:?}"),
        }
        assert!(input.is_empty());
        Ok(())
    }

    #[test]
    fn set_held_item_roundtrip() -> Result<(), PlayError> {
        let packet = SetHeldItem { slot: 4 };
        let mut wire = Vec::new();
        encode_set_held_item(&packet, TEST_MAX_FRAME, &mut wire)?;
        let mut input = wire.as_slice();
        match decode_set_held_item(&mut input, TEST_MAX_FRAME)? {
            PlayDecodeOutcome::Complete(PlayPacket::SetHeldItem(si)) => {
                assert_eq!(si.slot, 4);
            }
            other => panic!("expected SetHeldItem, got {other:?}"),
        }
        assert!(input.is_empty());
        Ok(())
    }

    #[test]
    fn entity_event_roundtrip() -> Result<(), PlayError> {
        let packet = EntityEvent {
            entity_id: 42,
            entity_status: 28,
        };
        let mut wire = Vec::new();
        encode_entity_event(&packet, TEST_MAX_FRAME, &mut wire)?;
        let mut input = wire.as_slice();
        match decode_entity_event(&mut input, TEST_MAX_FRAME)? {
            PlayDecodeOutcome::Complete(PlayPacket::EntityEvent(ee)) => {
                assert_eq!(ee.entity_id, 42);
                assert_eq!(ee.entity_status, 28);
            }
            other => panic!("expected EntityEvent, got {other:?}"),
        }
        assert!(input.is_empty());
        Ok(())
    }

    #[test]
    fn game_event_roundtrip() -> Result<(), PlayError> {
        let packet = GameEvent {
            event_type: 13, // Start waiting for level chunks
            value: 0.0,
        };
        let mut wire = Vec::new();
        encode_game_event(&packet, TEST_MAX_FRAME, &mut wire)?;
        let mut input = wire.as_slice();
        match decode_game_event(&mut input, TEST_MAX_FRAME)? {
            PlayDecodeOutcome::Complete(PlayPacket::GameEvent(ge)) => {
                assert_eq!(ge.event_type, 13);
                assert_eq!(ge.value, 0.0);
            }
            other => panic!("expected GameEvent, got {other:?}"),
        }
        assert!(input.is_empty());
        Ok(())
    }

    #[test]
    fn set_default_spawn_position_roundtrip() -> Result<(), PlayError> {
        let packet = SetDefaultSpawnPosition {
            location: (0, 64, 0),
            angle: 0.0,
        };
        let mut wire = Vec::new();
        encode_set_default_spawn_position(&packet, TEST_MAX_FRAME, &mut wire)?;
        let mut input = wire.as_slice();
        match decode_set_default_spawn_position(&mut input, TEST_MAX_FRAME)? {
            PlayDecodeOutcome::Complete(PlayPacket::SetDefaultSpawnPosition(sp)) => {
                assert_eq!(sp.location, (0, 64, 0));
                assert_eq!(sp.angle, 0.0);
            }
            other => panic!("expected SetDefaultSpawnPosition, got {other:?}"),
        }
        assert!(input.is_empty());
        Ok(())
    }

    #[test]
    fn set_center_chunk_roundtrip() -> Result<(), PlayError> {
        let packet = SetCenterChunk {
            chunk_x: 10,
            chunk_z: -5,
        };
        let mut wire = Vec::new();
        encode_set_center_chunk(&packet, TEST_MAX_FRAME, &mut wire)?;
        let mut input = wire.as_slice();
        match decode_set_center_chunk(&mut input, TEST_MAX_FRAME)? {
            PlayDecodeOutcome::Complete(PlayPacket::SetCenterChunk(sc)) => {
                assert_eq!(sc.chunk_x, 10);
                assert_eq!(sc.chunk_z, -5);
            }
            other => panic!("expected SetCenterChunk, got {other:?}"),
        }
        assert!(input.is_empty());
        Ok(())
    }

    #[test]
    fn unload_chunk_roundtrip() -> Result<(), PlayError> {
        let packet = UnloadChunk {
            chunk_x: 10,
            chunk_z: -5,
        };
        let mut wire = Vec::new();
        encode_unload_chunk(&packet, TEST_MAX_FRAME, &mut wire)?;
        let mut input = wire.as_slice();
        match decode_unload_chunk(&mut input, TEST_MAX_FRAME)? {
            PlayDecodeOutcome::Complete(PlayPacket::UnloadChunk(uc)) => {
                assert_eq!(uc.chunk_x, 10);
                assert_eq!(uc.chunk_z, -5);
            }
            other => panic!("expected UnloadChunk, got {other:?}"),
        }
        assert!(input.is_empty());
        Ok(())
    }

    #[test]
    fn unload_chunk_negative_coords_roundtrip() -> Result<(), PlayError> {
        let packet = UnloadChunk {
            chunk_x: -100,
            chunk_z: -200,
        };
        let mut wire = Vec::new();
        encode_unload_chunk(&packet, TEST_MAX_FRAME, &mut wire)?;
        let mut input = wire.as_slice();
        match decode_unload_chunk(&mut input, TEST_MAX_FRAME)? {
            PlayDecodeOutcome::Complete(PlayPacket::UnloadChunk(uc)) => {
                assert_eq!(uc.chunk_x, -100);
                assert_eq!(uc.chunk_z, -200);
            }
            other => panic!("expected UnloadChunk, got {other:?}"),
        }
        assert!(input.is_empty());
        Ok(())
    }

    #[test]
    fn unload_chunk_zero_coords_roundtrip() -> Result<(), PlayError> {
        let packet = UnloadChunk {
            chunk_x: 0,
            chunk_z: 0,
        };
        let mut wire = Vec::new();
        encode_unload_chunk(&packet, TEST_MAX_FRAME, &mut wire)?;
        let mut input = wire.as_slice();
        match decode_unload_chunk(&mut input, TEST_MAX_FRAME)? {
            PlayDecodeOutcome::Complete(PlayPacket::UnloadChunk(uc)) => {
                assert_eq!(uc.chunk_x, 0);
                assert_eq!(uc.chunk_z, 0);
            }
            other => panic!("expected UnloadChunk, got {other:?}"),
        }
        assert!(input.is_empty());
        Ok(())
    }

    #[test]
    fn combat_death_roundtrip() -> Result<(), PlayError> {
        let packet = CombatDeath {
            player_id: 42,
            message: r#"{"text":"Player was slain"}"#.to_string(),
        };
        let mut wire = Vec::new();
        encode_combat_death(&packet, TEST_MAX_FRAME, &mut wire)?;
        let mut input = wire.as_slice();
        match decode_combat_death(&mut input, TEST_MAX_FRAME)? {
            PlayDecodeOutcome::Complete(PlayPacket::CombatDeath(cd)) => {
                assert_eq!(cd.player_id, 42);
                assert_eq!(cd.message, packet.message);
            }
            other => panic!("expected CombatDeath, got {other:?}"),
        }
        assert!(input.is_empty());
        Ok(())
    }

    #[test]
    fn set_block_destroy_stage_roundtrip() -> Result<(), PlayError> {
        let packet = SetBlockDestroyStage {
            entity_id: 100,
            position: (10, -64, 20),
            destroy_stage: 5,
        };
        let mut wire = Vec::new();
        encode_set_block_destroy_stage(&packet, TEST_MAX_FRAME, &mut wire)?;
        let mut input = wire.as_slice();
        match decode_set_block_destroy_stage(&mut input, TEST_MAX_FRAME)? {
            PlayDecodeOutcome::Complete(PlayPacket::SetBlockDestroyStage(s)) => {
                assert_eq!(s.entity_id, 100);
                assert_eq!(s.position, (10, -64, 20));
                assert_eq!(s.destroy_stage, 5);
            }
            other => panic!("expected SetBlockDestroyStage, got {other:?}"),
        }
        assert!(input.is_empty());
        Ok(())
    }

    #[test]
    fn set_render_distance_roundtrip() -> Result<(), PlayError> {
        let packet = SetRenderDistance { view_distance: 10 };
        let mut wire = Vec::new();
        encode_set_render_distance(&packet, TEST_MAX_FRAME, &mut wire)?;
        let mut input = wire.as_slice();
        match decode_set_render_distance(&mut input, TEST_MAX_FRAME)? {
            PlayDecodeOutcome::Complete(PlayPacket::SetRenderDistance(rd)) => {
                assert_eq!(rd.view_distance, 10);
            }
            other => panic!("expected SetRenderDistance, got {other:?}"),
        }
        assert!(input.is_empty());
        Ok(())
    }

    #[test]
    fn set_simulation_distance_roundtrip() -> Result<(), PlayError> {
        let packet = SetSimulationDistance {
            simulation_distance: 8,
        };
        let mut wire = Vec::new();
        encode_set_simulation_distance(&packet, TEST_MAX_FRAME, &mut wire)?;
        let mut input = wire.as_slice();
        match decode_set_simulation_distance(&mut input, TEST_MAX_FRAME)? {
            PlayDecodeOutcome::Complete(PlayPacket::SetSimulationDistance(sd)) => {
                assert_eq!(sd.simulation_distance, 8);
            }
            other => panic!("expected SetSimulationDistance, got {other:?}"),
        }
        assert!(input.is_empty());
        Ok(())
    }

    // --- Multiplayer packets ---

    #[test]
    fn player_info_update_encode_add_player() -> Result<(), PlayError> {
        let packet = PlayerInfoUpdate {
            actions: PlayerInfoActions::new(PlayerInfoActions::ADD_PLAYER),
            entries: vec![PlayerInfoEntry {
                uuid: crate::primitives::Uuid::new(0x1234, 0x5678),
                name: "TestPlayer".to_string(),
                properties: vec![],
                gamemode: 0,
                listed: true,
                latency: 50,
                display_name: None,
            }],
        };
        let mut wire = Vec::new();
        encode_player_info_update(&packet, TEST_MAX_FRAME, &mut wire)?;
        assert!(!wire.is_empty());
        Ok(())
    }

    #[test]
    fn player_info_remove_encode() -> Result<(), PlayError> {
        let packet = PlayerInfoRemove {
            uuids: vec![crate::primitives::Uuid::new(0x1234, 0x5678)],
        };
        let mut wire = Vec::new();
        encode_player_info_remove(&packet, TEST_MAX_FRAME, &mut wire)?;
        assert!(!wire.is_empty());
        Ok(())
    }

    #[test]
    fn spawn_player_encode() -> Result<(), PlayError> {
        let packet = SpawnPlayer {
            entity_id: 42,
            uuid: crate::primitives::Uuid::new(0x1234, 0x5678),
            x: 128.5,
            y: 64.0,
            z: -32.0,
            yaw: 90,
            pitch: 0,
        };
        let mut wire = Vec::new();
        encode_spawn_player(&packet, TEST_MAX_FRAME, &mut wire)?;
        assert!(!wire.is_empty());
        Ok(())
    }

    #[test]
    fn remove_entities_encode() -> Result<(), PlayError> {
        let packet = RemoveEntities {
            entity_ids: vec![1, 2, 3],
        };
        let mut wire = Vec::new();
        encode_remove_entities(&packet, TEST_MAX_FRAME, &mut wire)?;
        assert!(!wire.is_empty());
        Ok(())
    }

    #[test]
    fn player_info_actions_flags() {
        let actions = PlayerInfoActions::new(
            PlayerInfoActions::ADD_PLAYER | PlayerInfoActions::UPDATE_LATENCY,
        );
        assert!(actions.has(PlayerInfoActions::ADD_PLAYER));
        assert!(actions.has(PlayerInfoActions::UPDATE_LATENCY));
        assert!(!actions.has(PlayerInfoActions::UPDATE_GAMEMODE));
    }

    // --- Block interaction packets ---

    #[test]
    fn player_digging_roundtrip() -> Result<(), PlayError> {
        let packet = PlayerDigging {
            action: PlayerDiggingAction::StartDestroy,
            position: (10, 64, -20),
            face: 1,
            sequence: 0,
        };
        let mut wire = Vec::new();
        encode_player_digging(&packet, TEST_MAX_FRAME, &mut wire)?;
        let mut input = wire.as_slice();
        match decode_player_digging(&mut input, TEST_MAX_FRAME)? {
            PlayDecodeOutcome::Complete(PlayPacket::PlayerDigging(decoded)) => {
                assert_eq!(decoded.action, PlayerDiggingAction::StartDestroy);
                assert_eq!(decoded.position, (10, 64, -20));
                assert_eq!(decoded.face, 1);
            }
            other => panic!("expected PlayerDigging, got {other:?}"),
        }
        assert!(input.is_empty());
        Ok(())
    }

    #[test]
    fn use_item_on_roundtrip() -> Result<(), PlayError> {
        let packet = UseItemOn {
            position: (0, 64, 0),
            face: 2,
            hand: 0,
            cursor_x: 0.5,
            cursor_y: 0.5,
            cursor_z: 0.5,
            inside_block: false,
            sequence: 0,
        };
        let mut wire = Vec::new();
        encode_use_item_on(&packet, TEST_MAX_FRAME, &mut wire)?;
        let mut input = wire.as_slice();
        match decode_use_item_on(&mut input, TEST_MAX_FRAME)? {
            PlayDecodeOutcome::Complete(PlayPacket::UseItemOn(decoded)) => {
                assert_eq!(decoded.position, (0, 64, 0));
                assert_eq!(decoded.face, 2);
                assert_eq!(decoded.hand, 0);
                assert_eq!(decoded.cursor_x, 0.5);
                assert!(!decoded.inside_block);
            }
            other => panic!("expected UseItemOn, got {other:?}"),
        }
        assert!(input.is_empty());
        Ok(())
    }

    #[test]
    fn block_update_roundtrip() -> Result<(), PlayError> {
        let packet = BlockUpdate {
            position: (10, 64, -20),
            block_state: 1, // stone
        };
        let mut wire = Vec::new();
        encode_block_update(&packet, TEST_MAX_FRAME, &mut wire)?;
        let mut input = wire.as_slice();
        match decode_block_update(&mut input, TEST_MAX_FRAME)? {
            PlayDecodeOutcome::Complete(PlayPacket::BlockUpdate(decoded)) => {
                assert_eq!(decoded.position, (10, 64, -20));
                assert_eq!(decoded.block_state, 1);
            }
            other => panic!("expected BlockUpdate, got {other:?}"),
        }
        assert!(input.is_empty());
        Ok(())
    }

    #[test]
    fn acknowledge_block_change_roundtrip() -> Result<(), PlayError> {
        let packet = AcknowledgeBlockChange { sequence: 42 };
        let mut wire = Vec::new();
        encode_acknowledge_block_change(&packet, TEST_MAX_FRAME, &mut wire)?;
        let mut input = wire.as_slice();
        match decode_acknowledge_block_change(&mut input, TEST_MAX_FRAME)? {
            PlayDecodeOutcome::Complete(PlayPacket::AcknowledgeBlockChange(decoded)) => {
                assert_eq!(decoded.sequence, 42);
            }
            other => panic!("expected AcknowledgeBlockChange, got {other:?}"),
        }
        assert!(input.is_empty());
        Ok(())
    }

    #[test]
    fn encode_update_tags_empty_has_expected_packet_id() -> Result<(), Box<dyn std::error::Error>> {
        let mut wire = Vec::new();
        encode_update_tags_empty(TEST_MAX_FRAME, &mut wire)?;
        let mut input: &[u8] = &wire;
        match crate::framing::decode_frame(&mut input, TEST_MAX_FRAME)? {
            crate::framing::DecodeOutcome::Complete(frame) => {
                assert_eq!(frame.packet_id, UPDATE_TAGS_PACKET_ID);
            }
            crate::framing::DecodeOutcome::Incomplete => panic!("expected complete frame"),
        }
        assert!(input.is_empty());
        Ok(())
    }

    #[test]
    fn encode_update_recipes_empty_has_expected_packet_id() -> Result<(), Box<dyn std::error::Error>>
    {
        let mut wire = Vec::new();
        encode_update_recipes_empty(TEST_MAX_FRAME, &mut wire)?;
        let mut input: &[u8] = &wire;
        match crate::framing::decode_frame(&mut input, TEST_MAX_FRAME)? {
            crate::framing::DecodeOutcome::Complete(frame) => {
                assert_eq!(frame.packet_id, UPDATE_RECIPES_PACKET_ID);
            }
            crate::framing::DecodeOutcome::Incomplete => panic!("expected complete frame"),
        }
        Ok(())
    }

    #[test]
    fn player_digging_action_from_wire() {
        assert_eq!(
            PlayerDiggingAction::from_wire(0),
            Some(PlayerDiggingAction::StartDestroy)
        );
        assert_eq!(
            PlayerDiggingAction::from_wire(3),
            Some(PlayerDiggingAction::DropAllItems)
        );
        assert_eq!(PlayerDiggingAction::from_wire(99), None);
    }

    // --- Entity movement packets ---

    #[test]
    fn move_entity_pos_roundtrip() -> Result<(), PlayError> {
        let packet = MoveEntityPos {
            entity_id: 42,
            delta_x: 100,
            delta_y: -200,
            delta_z: 300,
            on_ground: true,
        };
        let mut wire = Vec::new();
        encode_move_entity_pos(&packet, TEST_MAX_FRAME, &mut wire)?;
        let mut input = wire.as_slice();
        match decode_move_entity_pos(&mut input, TEST_MAX_FRAME)? {
            PlayDecodeOutcome::Complete(PlayPacket::MoveEntityPos(decoded)) => {
                assert_eq!(decoded.entity_id, 42);
                assert_eq!(decoded.delta_x, 100);
                assert_eq!(decoded.delta_y, -200);
                assert_eq!(decoded.delta_z, 300);
                assert!(decoded.on_ground);
            }
            other => panic!("expected MoveEntityPos, got {other:?}"),
        }
        assert!(input.is_empty());
        Ok(())
    }

    #[test]
    fn move_entity_pos_rot_roundtrip() -> Result<(), PlayError> {
        let packet = MoveEntityPosRot {
            entity_id: 7,
            delta_x: 4096, // 1 block
            delta_y: 0,
            delta_z: -4096, // -1 block
            yaw: 128,       // 180 degrees
            pitch: 64,      // 90 degrees
            on_ground: false,
        };
        let mut wire = Vec::new();
        encode_move_entity_pos_rot(&packet, TEST_MAX_FRAME, &mut wire)?;
        let mut input = wire.as_slice();
        match decode_move_entity_pos_rot(&mut input, TEST_MAX_FRAME)? {
            PlayDecodeOutcome::Complete(PlayPacket::MoveEntityPosRot(decoded)) => {
                assert_eq!(decoded.entity_id, 7);
                assert_eq!(decoded.delta_x, 4096);
                assert_eq!(decoded.delta_y, 0);
                assert_eq!(decoded.delta_z, -4096);
                assert_eq!(decoded.yaw, 128);
                assert_eq!(decoded.pitch, 64);
                assert!(!decoded.on_ground);
            }
            other => panic!("expected MoveEntityPosRot, got {other:?}"),
        }
        assert!(input.is_empty());
        Ok(())
    }

    #[test]
    fn move_entity_rot_roundtrip() -> Result<(), PlayError> {
        let packet = MoveEntityRot {
            entity_id: 99,
            yaw: 0,
            pitch: 255,
            on_ground: true,
        };
        let mut wire = Vec::new();
        encode_move_entity_rot(&packet, TEST_MAX_FRAME, &mut wire)?;
        let mut input = wire.as_slice();
        match decode_move_entity_rot(&mut input, TEST_MAX_FRAME)? {
            PlayDecodeOutcome::Complete(PlayPacket::MoveEntityRot(decoded)) => {
                assert_eq!(decoded.entity_id, 99);
                assert_eq!(decoded.yaw, 0);
                assert_eq!(decoded.pitch, 255);
                assert!(decoded.on_ground);
            }
            other => panic!("expected MoveEntityRot, got {other:?}"),
        }
        assert!(input.is_empty());
        Ok(())
    }

    #[test]
    fn entity_teleport_roundtrip() -> Result<(), PlayError> {
        let packet = EntityTeleport {
            entity_id: 123,
            x: 100.5,
            y: -64.0,
            z: 0.25,
            yaw: 64, // 90 degrees
            pitch: 0,
            on_ground: true,
        };
        let mut wire = Vec::new();
        encode_entity_teleport(&packet, TEST_MAX_FRAME, &mut wire)?;
        let mut input = wire.as_slice();
        match decode_entity_teleport(&mut input, TEST_MAX_FRAME)? {
            PlayDecodeOutcome::Complete(PlayPacket::EntityTeleport(decoded)) => {
                assert_eq!(decoded.entity_id, 123);
                assert_eq!(decoded.x, 100.5);
                assert_eq!(decoded.y, -64.0);
                assert_eq!(decoded.z, 0.25);
                assert_eq!(decoded.yaw, 64);
                assert_eq!(decoded.pitch, 0);
                assert!(decoded.on_ground);
            }
            other => panic!("expected EntityTeleport, got {other:?}"),
        }
        assert!(input.is_empty());
        Ok(())
    }

    #[test]
    fn move_entity_pos_max_delta() -> Result<(), PlayError> {
        let packet = MoveEntityPos {
            entity_id: 1,
            delta_x: 32767,
            delta_y: -32768,
            delta_z: 0,
            on_ground: false,
        };
        let mut wire = Vec::new();
        encode_move_entity_pos(&packet, TEST_MAX_FRAME, &mut wire)?;
        let mut input = wire.as_slice();
        match decode_move_entity_pos(&mut input, TEST_MAX_FRAME)? {
            PlayDecodeOutcome::Complete(PlayPacket::MoveEntityPos(decoded)) => {
                assert_eq!(decoded.delta_x, 32767);
                assert_eq!(decoded.delta_y, -32768);
            }
            other => panic!("expected MoveEntityPos, got {other:?}"),
        }
        Ok(())
    }

    #[test]
    fn slot_empty_roundtrip() {
        let slot = Slot::empty();
        let mut wire = Vec::new();
        encode_slot(&slot, &mut wire);
        assert_eq!(wire, vec![0x00]); // just bool false
        let mut input = wire.as_slice();
        let decoded = decode_slot(&mut input).unwrap_or_else(|_| panic!("decode failed"));
        assert!(!decoded.present);
        assert!(input.is_empty());
    }

    #[test]
    fn slot_item_no_nbt_roundtrip() {
        let slot = Slot::item(10, 64); // item ID 10, count 64
        let mut wire = Vec::new();
        encode_slot(&slot, &mut wire);
        let mut input = wire.as_slice();
        let decoded = decode_slot(&mut input).unwrap_or_else(|_| panic!("decode failed"));
        assert!(decoded.present);
        assert_eq!(decoded.item_id, 10);
        assert_eq!(decoded.count, 64);
        assert!(decoded.nbt.is_empty());
        assert!(input.is_empty());
    }

    #[test]
    fn slot_item_with_nbt_roundtrip() {
        // Build a minimal NBT compound: TAG_Compound + empty name + TAG_End
        let nbt = vec![0x0A, 0x00, 0x00, 0x00]; // compound, name len=0, TAG_End
        let slot = Slot::with_nbt(5, 1, nbt.clone());
        let mut wire = Vec::new();
        encode_slot(&slot, &mut wire);
        let mut input = wire.as_slice();
        let decoded = decode_slot(&mut input).unwrap_or_else(|_| panic!("decode failed"));
        assert!(decoded.present);
        assert_eq!(decoded.item_id, 5);
        assert_eq!(decoded.count, 1);
        assert_eq!(decoded.nbt, nbt);
        assert!(input.is_empty());
    }

    #[test]
    fn set_container_content_roundtrip() -> Result<(), PlayError> {
        let packet = SetContainerContent {
            window_id: 0,
            state_id: 42,
            slots: vec![Slot::item(1, 64), Slot::empty(), Slot::item(2, 32)],
            carried_item: Slot::empty(),
        };
        let mut wire = Vec::new();
        encode_set_container_content(&packet, TEST_MAX_FRAME, &mut wire)?;
        let mut input = wire.as_slice();
        match decode_set_container_content(&mut input, TEST_MAX_FRAME)? {
            PlayDecodeOutcome::Complete(PlayPacket::SetContainerContent(decoded)) => {
                assert_eq!(decoded.window_id, 0);
                assert_eq!(decoded.state_id, 42);
                assert_eq!(decoded.slots.len(), 3);
                assert!(decoded.slots[0].present);
                assert_eq!(decoded.slots[0].item_id, 1);
                assert_eq!(decoded.slots[0].count, 64);
                assert!(!decoded.slots[1].present);
                assert!(decoded.slots[2].present);
                assert_eq!(decoded.slots[2].item_id, 2);
                assert!(!decoded.carried_item.present);
            }
            other => panic!("expected SetContainerContent, got {other:?}"),
        }
        assert!(input.is_empty());
        Ok(())
    }

    #[test]
    fn set_container_content_empty_roundtrip() -> Result<(), PlayError> {
        let packet = SetContainerContent {
            window_id: 0,
            state_id: 0,
            slots: vec![],
            carried_item: Slot::empty(),
        };
        let mut wire = Vec::new();
        encode_set_container_content(&packet, TEST_MAX_FRAME, &mut wire)?;
        let mut input = wire.as_slice();
        match decode_set_container_content(&mut input, TEST_MAX_FRAME)? {
            PlayDecodeOutcome::Complete(PlayPacket::SetContainerContent(decoded)) => {
                assert_eq!(decoded.slots.len(), 0);
            }
            other => panic!("expected SetContainerContent, got {other:?}"),
        }
        Ok(())
    }

    #[test]
    fn set_container_slot_roundtrip() -> Result<(), PlayError> {
        let packet = SetContainerSlot {
            window_id: 0,
            state_id: 1,
            slot: 36,
            item: Slot::item(10, 1),
        };
        let mut wire = Vec::new();
        encode_set_container_slot(&packet, TEST_MAX_FRAME, &mut wire)?;
        let mut input = wire.as_slice();
        match decode_set_container_slot(&mut input, TEST_MAX_FRAME)? {
            PlayDecodeOutcome::Complete(PlayPacket::SetContainerSlot(decoded)) => {
                assert_eq!(decoded.window_id, 0);
                assert_eq!(decoded.state_id, 1);
                assert_eq!(decoded.slot, 36);
                assert!(decoded.item.present);
                assert_eq!(decoded.item.item_id, 10);
            }
            other => panic!("expected SetContainerSlot, got {other:?}"),
        }
        assert!(input.is_empty());
        Ok(())
    }

    #[test]
    fn set_container_slot_clear_roundtrip() -> Result<(), PlayError> {
        let packet = SetContainerSlot {
            window_id: 0,
            state_id: 1,
            slot: 0,
            item: Slot::empty(),
        };
        let mut wire = Vec::new();
        encode_set_container_slot(&packet, TEST_MAX_FRAME, &mut wire)?;
        let mut input = wire.as_slice();
        match decode_set_container_slot(&mut input, TEST_MAX_FRAME)? {
            PlayDecodeOutcome::Complete(PlayPacket::SetContainerSlot(decoded)) => {
                assert!(!decoded.item.present);
            }
            other => panic!("expected SetContainerSlot, got {other:?}"),
        }
        Ok(())
    }

    #[test]
    fn set_held_item_serverbound_roundtrip() -> Result<(), PlayError> {
        let mut body = Vec::new();
        crate::primitives::encode_i16(5, &mut body);
        let mut wire = Vec::new();
        crate::framing::encode_frame(
            SET_HELD_ITEM_SERVERBOUND_PACKET_ID,
            &body,
            TEST_MAX_FRAME,
            &mut wire,
        )
        .map_err(PlayError::from)?;

        let mut input = wire.as_slice();
        match decode_set_held_item_serverbound(&mut input, TEST_MAX_FRAME)? {
            PlayDecodeOutcome::Complete(PlayPacket::SetHeldItemServerbound(decoded)) => {
                assert_eq!(decoded.slot, 5);
            }
            other => panic!("expected SetHeldItemServerbound, got {other:?}"),
        }
        assert!(input.is_empty());
        Ok(())
    }

    #[test]
    fn set_creative_mode_slot_roundtrip() -> Result<(), PlayError> {
        // Build a Set Creative Mode Slot packet manually
        let mut body = Vec::new();
        crate::primitives::encode_i16(36, &mut body); // slot
        crate::primitives::encode_bool(true, &mut body); // present
        crate::primitives::encode_var_int(10, &mut body); // item_id
        crate::primitives::encode_i8(64, &mut body); // count
        body.push(0x00); // TAG_End (no NBT)

        let mut wire = Vec::new();
        crate::framing::encode_frame(
            SET_CREATIVE_MODE_SLOT_PACKET_ID,
            &body,
            TEST_MAX_FRAME,
            &mut wire,
        )
        .map_err(PlayError::from)?;

        let mut input = wire.as_slice();
        match decode_set_creative_mode_slot(&mut input, TEST_MAX_FRAME)? {
            PlayDecodeOutcome::Complete(PlayPacket::SetCreativeModeSlot(decoded)) => {
                assert_eq!(decoded.slot, 36);
                assert!(decoded.item.present);
                assert_eq!(decoded.item.item_id, 10);
                assert_eq!(decoded.item.count, 64);
            }
            other => panic!("expected SetCreativeModeSlot, got {other:?}"),
        }
        assert!(input.is_empty());
        Ok(())
    }

    #[test]
    fn set_creative_mode_slot_drop_item() -> Result<(), PlayError> {
        // Slot -1 means dropping the item
        let mut body = Vec::new();
        crate::primitives::encode_i16(-1, &mut body); // slot = -1 (drop)
        crate::primitives::encode_bool(true, &mut body);
        crate::primitives::encode_var_int(1, &mut body);
        crate::primitives::encode_i8(1, &mut body);
        body.push(0x00); // no NBT

        let mut wire = Vec::new();
        crate::framing::encode_frame(
            SET_CREATIVE_MODE_SLOT_PACKET_ID,
            &body,
            TEST_MAX_FRAME,
            &mut wire,
        )
        .map_err(PlayError::from)?;

        let mut input = wire.as_slice();
        match decode_set_creative_mode_slot(&mut input, TEST_MAX_FRAME)? {
            PlayDecodeOutcome::Complete(PlayPacket::SetCreativeModeSlot(decoded)) => {
                assert_eq!(decoded.slot, -1);
                assert!(decoded.item.present);
            }
            other => panic!("expected SetCreativeModeSlot, got {other:?}"),
        }
        Ok(())
    }

    #[test]
    fn set_creative_mode_slot_clear_slot() -> Result<(), PlayError> {
        // Picking up an item from creative inventory sends present=false
        let mut body = Vec::new();
        crate::primitives::encode_i16(10, &mut body);
        crate::primitives::encode_bool(false, &mut body); // empty slot

        let mut wire = Vec::new();
        crate::framing::encode_frame(
            SET_CREATIVE_MODE_SLOT_PACKET_ID,
            &body,
            TEST_MAX_FRAME,
            &mut wire,
        )
        .map_err(PlayError::from)?;

        let mut input = wire.as_slice();
        match decode_set_creative_mode_slot(&mut input, TEST_MAX_FRAME)? {
            PlayDecodeOutcome::Complete(PlayPacket::SetCreativeModeSlot(decoded)) => {
                assert_eq!(decoded.slot, 10);
                assert!(!decoded.item.present);
            }
            other => panic!("expected SetCreativeModeSlot, got {other:?}"),
        }
        Ok(())
    }

    #[test]
    fn system_chat_message_roundtrip() -> Result<(), PlayError> {
        let packet = SystemChatMessage {
            content: r#"{"text":"Hello, world!"}"#.to_string(),
            overlay: false,
        };
        let mut wire = Vec::new();
        encode_system_chat_message(&packet, TEST_MAX_FRAME, &mut wire)?;
        // Verify the packet ID is correct
        let mut input = wire.as_slice();
        let frame =
            crate::framing::decode_frame(&mut input, TEST_MAX_FRAME).map_err(PlayError::from)?;
        match frame {
            crate::framing::DecodeOutcome::Complete(f) => {
                assert_eq!(f.packet_id, SYSTEM_CHAT_MESSAGE_PACKET_ID);
                let mut body = f.payload;
                let content = decode_string(&mut body, 32767).map_err(PlayError::Codec)?;
                assert_eq!(content, r#"{"text":"Hello, world!"}"#);
                let overlay = decode_bool(&mut body).map_err(PlayError::Codec)?;
                assert!(!overlay);
                assert!(body.is_empty());
            }
            crate::framing::DecodeOutcome::Incomplete => panic!("expected complete frame"),
        }
        Ok(())
    }

    #[test]
    fn system_chat_message_overlay() -> Result<(), PlayError> {
        let packet = SystemChatMessage {
            content: r#"{"text":"Game tip!"}"#.to_string(),
            overlay: true,
        };
        let mut wire = Vec::new();
        encode_system_chat_message(&packet, TEST_MAX_FRAME, &mut wire)?;
        let mut input = wire.as_slice();
        let frame =
            crate::framing::decode_frame(&mut input, TEST_MAX_FRAME).map_err(PlayError::from)?;
        match frame {
            crate::framing::DecodeOutcome::Complete(f) => {
                assert_eq!(f.packet_id, SYSTEM_CHAT_MESSAGE_PACKET_ID);
                let mut body = f.payload;
                let _content = decode_string(&mut body, 32767).map_err(PlayError::Codec)?;
                let overlay = decode_bool(&mut body).map_err(PlayError::Codec)?;
                assert!(overlay);
            }
            crate::framing::DecodeOutcome::Incomplete => panic!("expected complete frame"),
        }
        Ok(())
    }

    #[test]
    fn decode_chat_message_serverbound_extracts_message() -> Result<(), PlayError> {
        // Build a Chat Message serverbound packet manually.
        // Format: packet_id (0x01) + message_string + trailing_fields
        let mut body = Vec::new();
        crate::primitives::encode_string("Hello chat!", 256, &mut body).map_err(PlayError::from)?;
        // Add dummy trailing fields (timestamp, salt, signature, acks)
        // that would normally be present in a real chat packet.
        crate::primitives::encode_i64(1234567890, &mut body); // timestamp
        crate::primitives::encode_i64(0, &mut body); // salt
        crate::primitives::encode_bool(false, &mut body); // no signature
        // Minimal acknowledgements: count=0
        crate::primitives::encode_var_int(0, &mut body); // offset
        // 3 bytes of bitset for acknowledged (simplified)
        body.extend_from_slice(&[0, 0, 0]);

        let mut wire = Vec::new();
        crate::framing::encode_frame(
            CHAT_MESSAGE_SERVERBOUND_PACKET_ID,
            &body,
            TEST_MAX_FRAME,
            &mut wire,
        )
        .map_err(PlayError::from)?;

        let mut input = wire.as_slice();
        match decode_chat_message_serverbound(&mut input, TEST_MAX_FRAME)? {
            PlayDecodeOutcome::Complete(PlayPacket::ChatMessageServerbound(chat)) => {
                assert_eq!(chat.message, "Hello chat!");
            }
            other => panic!("expected ChatMessageServerbound, got {other:?}"),
        }
        assert!(input.is_empty());
        Ok(())
    }

    #[test]
    fn set_health_roundtrip() -> Result<(), PlayError> {
        let packet = SetHealth {
            health: 20.0,
            food: 20,
            food_saturation: 5.0,
        };
        let mut wire = Vec::new();
        encode_set_health(&packet, TEST_MAX_FRAME, &mut wire)?;
        let mut input = wire.as_slice();
        match decode_set_health(&mut input, TEST_MAX_FRAME)? {
            PlayDecodeOutcome::Complete(PlayPacket::SetHealth(decoded)) => {
                assert_eq!(decoded.health, 20.0);
                assert_eq!(decoded.food, 20);
                assert_eq!(decoded.food_saturation, 5.0);
            }
            other => panic!("expected SetHealth, got {other:?}"),
        }
        assert!(input.is_empty());
        Ok(())
    }

    #[test]
    fn decode_chat_message_wrong_id_returns_error() -> Result<(), PlayError> {
        // Build a frame with wrong packet ID
        let body = vec![0u8; 10];
        let mut wire = Vec::new();
        crate::framing::encode_frame(0x99, &body, TEST_MAX_FRAME, &mut wire)
            .map_err(PlayError::from)?;

        let mut input = wire.as_slice();
        let result = decode_chat_message_serverbound(&mut input, TEST_MAX_FRAME);
        assert!(result.is_err());
        match result {
            Err(PlayError::WrongPacketId { received, expected }) => {
                assert_eq!(received, 0x99);
                assert_eq!(expected, CHAT_MESSAGE_SERVERBOUND_PACKET_ID);
            }
            other => panic!("expected WrongPacketId, got {other:?}"),
        }
        Ok(())
    }

    #[test]
    fn set_health_dead_roundtrip() -> Result<(), PlayError> {
        let packet = SetHealth {
            health: 0.0,
            food: 0,
            food_saturation: 0.0,
        };
        let mut wire = Vec::new();
        encode_set_health(&packet, TEST_MAX_FRAME, &mut wire)?;
        let mut input = wire.as_slice();
        match decode_set_health(&mut input, TEST_MAX_FRAME)? {
            PlayDecodeOutcome::Complete(PlayPacket::SetHealth(decoded)) => {
                assert_eq!(decoded.health, 0.0);
                assert_eq!(decoded.food, 0);
            }
            other => panic!("expected SetHealth, got {other:?}"),
        }
        Ok(())
    }

    #[test]
    fn respawn_roundtrip_no_death() -> Result<(), PlayError> {
        let packet = Respawn {
            dimension_type: "minecraft:overworld".to_string(),
            dimension_name: "minecraft:overworld".to_string(),
            hashed_seed: 12345,
            gamemode: 1, // Creative
            previous_gamemode: -1,
            is_debug: false,
            is_flat: false,
            has_death_location: false,
            death_dimension_name: String::new(),
            death_location: (0, 0, 0),
            portal_cooldown: 0,
            data_kept: 0,
        };
        let mut wire = Vec::new();
        encode_respawn(&packet, TEST_MAX_FRAME, &mut wire)?;
        let mut input = wire.as_slice();
        match decode_respawn(&mut input, TEST_MAX_FRAME)? {
            PlayDecodeOutcome::Complete(PlayPacket::Respawn(decoded)) => {
                assert_eq!(decoded.dimension_type, "minecraft:overworld");
                assert_eq!(decoded.dimension_name, "minecraft:overworld");
                assert_eq!(decoded.hashed_seed, 12345);
                assert_eq!(decoded.gamemode, 1);
                assert_eq!(decoded.previous_gamemode, -1);
                assert!(!decoded.is_debug);
                assert!(!decoded.is_flat);
                assert!(!decoded.has_death_location);
                assert_eq!(decoded.portal_cooldown, 0);
                assert_eq!(decoded.data_kept, 0);
            }
            other => panic!("expected Respawn, got {other:?}"),
        }
        assert!(input.is_empty());
        Ok(())
    }

    #[test]
    fn respawn_roundtrip_with_death_location() -> Result<(), PlayError> {
        let packet = Respawn {
            dimension_type: "minecraft:overworld".to_string(),
            dimension_name: "minecraft:overworld".to_string(),
            hashed_seed: 0,
            gamemode: 0,
            previous_gamemode: 0,
            is_debug: false,
            is_flat: false,
            has_death_location: true,
            death_dimension_name: "minecraft:the_nether".to_string(),
            death_location: (10, 64, -20),
            portal_cooldown: 300,
            data_kept: 0,
        };
        let mut wire = Vec::new();
        encode_respawn(&packet, TEST_MAX_FRAME, &mut wire)?;
        let mut input = wire.as_slice();
        match decode_respawn(&mut input, TEST_MAX_FRAME)? {
            PlayDecodeOutcome::Complete(PlayPacket::Respawn(decoded)) => {
                assert!(decoded.has_death_location);
                assert_eq!(decoded.death_dimension_name, "minecraft:the_nether");
                assert_eq!(decoded.death_location, (10, 64, -20));
                assert_eq!(decoded.portal_cooldown, 300);
            }
            other => panic!("expected Respawn, got {other:?}"),
        }
        assert!(input.is_empty());
        Ok(())
    }

    #[test]
    fn client_status_respawn_roundtrip() -> Result<(), PlayError> {
        let mut body = Vec::new();
        crate::primitives::encode_var_int(0, &mut body); // action = Perform respawn
        let mut wire = Vec::new();
        crate::framing::encode_frame(CLIENT_STATUS_PACKET_ID, &body, TEST_MAX_FRAME, &mut wire)
            .map_err(PlayError::from)?;

        let mut input = wire.as_slice();
        match decode_client_status(&mut input, TEST_MAX_FRAME)? {
            PlayDecodeOutcome::Complete(PlayPacket::ClientStatus(decoded)) => {
                assert_eq!(decoded.action, 0);
            }
            other => panic!("expected ClientStatus, got {other:?}"),
        }
        assert!(input.is_empty());
        Ok(())
    }

    #[test]
    fn client_status_request_stats_roundtrip() -> Result<(), PlayError> {
        let mut body = Vec::new();
        crate::primitives::encode_var_int(1, &mut body); // action = Request stats
        let mut wire = Vec::new();
        crate::framing::encode_frame(CLIENT_STATUS_PACKET_ID, &body, TEST_MAX_FRAME, &mut wire)
            .map_err(PlayError::from)?;

        let mut input = wire.as_slice();
        match decode_client_status(&mut input, TEST_MAX_FRAME)? {
            PlayDecodeOutcome::Complete(PlayPacket::ClientStatus(decoded)) => {
                assert_eq!(decoded.action, 1);
            }
            other => panic!("expected ClientStatus, got {other:?}"),
        }
        Ok(())
    }
}
