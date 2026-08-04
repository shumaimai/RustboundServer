//! Player session module for the Rustbound server.
//!
//! A `PlayerSession` encapsulates the Play-state lifecycle for a single
//! connection: from Join Game through KeepAlive cycling to disconnect.
//! Each session owns a channel for receiving events from the tick loop
//! (e.g. KeepAlive) and uses a sender to forward state changes (position
//! updates, join/leave) back to the tick loop.

use std::fmt;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use rustbound_protocol::play::{
    ChangeDifficulty, ClientInformation, CombatDeath, ConfirmTeleportation, DisconnectPlay,
    EntityEvent, GameEvent, GameMode, JoinGame, KeepAlive, PlayDecodeOutcome, PlayError,
    PlayPacket, PlayerAbilities, PluginMessageClientbound, Respawn, SetBlockDestroyStage,
    SetCenterChunk, SetContainerContent, SetContainerSlot, SetDefaultSpawnPosition, SetHealth,
    SetHeldItem, SetRenderDistance, SetSimulationDistance, SynchronizePlayerPosition,
    SystemChatMessage, UnloadChunk, decode_chat_message_serverbound, decode_client_information,
    decode_client_status, decode_confirm_teleportation, decode_keep_alive_serverbound,
    decode_player_digging, decode_set_creative_mode_slot, decode_set_held_item_serverbound,
    decode_set_player_position, decode_set_player_position_and_rotation,
    decode_set_player_rotation, decode_use_item_on, encode_change_difficulty, encode_chunk_data,
    encode_combat_death, encode_disconnect_play, encode_entity_event, encode_game_event,
    encode_join_game, encode_keep_alive_clientbound, encode_player_abilities,
    encode_plugin_message_clientbound, encode_respawn, encode_set_block_destroy_stage,
    encode_set_center_chunk, encode_set_container_content, encode_set_container_slot,
    encode_set_default_spawn_position, encode_set_health, encode_set_held_item,
    encode_set_render_distance, encode_set_simulation_distance, encode_synchronize_player_position,
    encode_system_chat_message, encode_unload_chunk,
};
use rustbound_protocol::primitives::Uuid;

use crate::tick::TickMessage;

/// Converts a yaw/pitch angle in degrees to the protocol byte format.
///
/// Minecraft encodes angles as a single byte where 256 steps = 360 degrees.
/// The value wraps around (modulo 256).
fn degrees_to_angle_byte(degrees: f32) -> u8 {
    let normalized = degrees.rem_euclid(360.0);
    let steps = normalized / 360.0 * 256.0;
    steps.round() as u8
}

/// Player Abilities flags bitmask for the given gamemode.
///
/// bit0 invulnerable, bit1 flying, bit2 allow-flying, bit3 creative.
fn player_abilities_flags(gamemode: GameMode) -> u8 {
    match gamemode {
        GameMode::Creative => 0x0D,  // invulnerable + allow-fly + creative
        GameMode::Spectator => 0x07, // invulnerable + flying + allow-fly
        GameMode::Survival | GameMode::Adventure => 0x00,
    }
}

/// Movement data for a remote entity, used to choose the correct packet type.
#[derive(Debug, Clone, Copy)]
pub struct EntityMovementData {
    /// The moving entity's ID.
    pub entity_id: i32,
    /// Previous absolute X.
    pub old_x: f64,
    /// Previous absolute Y.
    pub old_y: f64,
    /// Previous absolute Z.
    pub old_z: f64,
    /// New absolute X.
    pub new_x: f64,
    /// New absolute Y.
    pub new_y: f64,
    /// New absolute Z.
    pub new_z: f64,
    /// New yaw (degrees).
    pub new_yaw: f32,
    /// New pitch (degrees).
    pub new_pitch: f32,
    /// Whether the entity is on the ground.
    pub on_ground: bool,
}

/// Events delivered from the tick loop to a player session.
#[derive(Debug, Clone)]
pub enum SessionEvent {
    /// A keep-alive should be sent to the client.
    KeepAlive {
        /// The keep-alive payload to echo.
        payload: i64,
    },
    /// A new player joined; send Player Info Update and Spawn Player.
    PlayerJoined {
        /// The joining player's entity ID.
        entity_id: i32,
        /// The joining player's UUID.
        uuid: Uuid,
        /// The joining player's username.
        username: String,
        /// The joining player's gamemode.
        gamemode: u8,
        /// The joining player's position.
        x: f64,
        y: f64,
        z: f64,
    },
    /// A player left; send Player Info Remove and Remove Entities.
    PlayerLeft {
        /// The leaving player's entity ID.
        entity_id: i32,
        /// The leaving player's UUID.
        uuid: Uuid,
    },
    /// A block was changed; send Block Update.
    BlockUpdate {
        /// The block position.
        position: (i32, i32, i32),
        /// The new block state ID (0 = air).
        block_state: i32,
    },
    /// Block overrides for a chunk that was just sent; send Block Update
    /// for each override so the player sees dug/placed blocks.
    ChunkBlockOverrides {
        /// List of (position, block_state) pairs to apply.
        overrides: Vec<((i32, i32, i32), i32)>,
    },
    /// A remote player moved and/or rotated; send the appropriate entity
    /// movement packet to this client.
    EntityMovement {
        /// The moving player's entity ID.
        entity_id: i32,
        /// Previous absolute X.
        old_x: f64,
        /// Previous absolute Y.
        old_y: f64,
        /// Previous absolute Z.
        old_z: f64,
        /// New absolute X.
        new_x: f64,
        /// New absolute Y.
        new_y: f64,
        /// New absolute Z.
        new_z: f64,
        /// New yaw (degrees).
        new_yaw: f32,
        /// New pitch (degrees).
        new_pitch: f32,
        /// Whether the player is on the ground.
        on_ground: bool,
    },
    /// Update the client's center chunk; send Set Center Chunk packet.
    SetCenterChunkEvent {
        /// The new center chunk X.
        chunk_x: i32,
        /// The new center chunk Z.
        chunk_z: i32,
    },
    /// Send a Chunk Data packet for a newly loaded chunk.
    LoadChunk {
        /// The chunk X coordinate.
        chunk_x: i32,
        /// The chunk Z coordinate.
        chunk_z: i32,
    },
    /// Send an Unload Chunk packet for a chunk that left the player's view.
    UnloadChunk {
        /// The chunk X coordinate.
        chunk_x: i32,
        /// The chunk Z coordinate.
        chunk_z: i32,
    },
    /// Send a Combat Death packet to display the death screen.
    CombatDeath {
        /// The entity ID of the player that died.
        player_id: i32,
        /// The death message as a JSON chat component string.
        message: String,
    },
    /// Send a Set Block Destroy Stage packet to show break progress.
    SetBlockDestroyStage {
        /// The entity ID breaking the block.
        entity_id: i32,
        /// The block position.
        position: (i32, i32, i32),
        /// The destroy stage (0–9 to set, any other to remove).
        destroy_stage: i8,
    },
    /// Send a Set Container Content packet to initialize or sync the inventory.
    SetContainerContent {
        /// The window ID (0 for player inventory).
        window_id: u8,
        /// Server-managed state ID.
        state_id: i32,
        /// The slot data for all slots.
        slots: Vec<rustbound_protocol::play::Slot>,
        /// The carried (cursor) item.
        carried_item: rustbound_protocol::play::Slot,
    },
    /// Send a Set Container Slot packet to update a single slot.
    SetContainerSlot {
        /// The window ID.
        window_id: i8,
        /// Server-managed state ID.
        state_id: i32,
        /// The slot index.
        slot: i16,
        /// The new slot data.
        item: rustbound_protocol::play::Slot,
    },
    /// Open a container GUI on the client.
    OpenScreen {
        /// Window ID (non-zero).
        window_id: i32,
        /// Menu type registry ID.
        window_type: i32,
        /// JSON chat title.
        title: String,
    },
    /// Force-close a container window on the client.
    CloseContainer {
        /// Window ID to close.
        window_id: u8,
    },
    /// Send a Set Held Item (clientbound) packet to sync the hotbar selection.
    SetHeldItemClientbound {
        /// The hotbar slot (0-8).
        slot: u8,
    },
    /// Send a system chat message to the client.
    SystemChat {
        /// The JSON chat component string.
        content: String,
    },
    /// Send a Set Health packet to the client.
    SetHealth {
        /// The player's health (0 or less = dead, 20 = full HP).
        health: f32,
        /// The player's food level (0-20).
        food: i32,
        /// The player's food saturation (0.0 to 5.0).
        food_saturation: f32,
    },
    /// Send a Respawn packet and re-synchronize the player after death.
    RespawnPlayer {
        /// The dimension type identifier.
        dimension_type: String,
        /// The dimension name identifier.
        dimension_name: String,
        /// The hashed seed.
        hashed_seed: i64,
        /// The player's gamemode.
        gamemode: u8,
        /// The previous gamemode (-1 = undefined).
        previous_gamemode: i8,
        /// Whether the world is debug.
        is_debug: bool,
        /// Whether the world is flat.
        is_flat: bool,
        /// Whether death location data is present.
        has_death_location: bool,
        /// The dimension name where the player died.
        death_dimension_name: String,
        /// The location where the player died.
        death_location: (i32, i32, i32),
        /// Portal cooldown in ticks.
        portal_cooldown: i32,
        /// Data kept bitmask.
        data_kept: u8,
        /// The new position after respawn (spawn point).
        x: f64,
        /// The new Y position.
        y: f64,
        /// The new Z position.
        z: f64,
    },
    /// Force the client to an absolute position (garden border clamp).
    SynchronizePosition {
        /// Absolute X.
        x: f64,
        /// Absolute Y.
        y: f64,
        /// Absolute Z.
        z: f64,
    },
    /// Spawn a non-player entity (mob) via Spawn Entity.
    SpawnMob {
        /// Entity ID.
        entity_id: i32,
        /// Entity UUID.
        uuid: Uuid,
        /// `minecraft:entity_type` registry ID.
        entity_type: i32,
        /// Feet X.
        x: f64,
        /// Feet Y.
        y: f64,
        /// Feet Z.
        z: f64,
        /// Yaw degrees.
        yaw: f32,
        /// Pitch degrees.
        pitch: f32,
    },
    /// Remove one or more entities from the client.
    RemoveEntities {
        /// Entity IDs to destroy.
        entity_ids: Vec<i32>,
    },
}

/// An error encountered while running a player session.
#[derive(Debug)]
pub enum SessionError {
    /// An I/O error occurred.
    Io(std::io::Error),
    /// The remote end closed the connection.
    Disconnected,
    /// A play protocol error occurred.
    Play(PlayError),
    /// A framing error occurred.
    Framing(rustbound_protocol::framing::FramingError),
    /// The client sent an unexpected packet.
    UnexpectedPacket(i32),
    /// The client did not confirm teleportation in time.
    TeleportTimeout,
    /// The client sent a position with NaN or Inf coordinates.
    InvalidPosition,
    /// The client did not respond to KeepAlive within the timeout.
    KeepAliveTimeout,
}

impl fmt::Display for SessionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "I/O error: {error}"),
            Self::Disconnected => formatter.write_str("remote disconnected"),
            Self::Play(error) => write!(formatter, "play error: {error}"),
            Self::Framing(error) => write!(formatter, "framing error: {error}"),
            Self::UnexpectedPacket(id) => {
                write!(formatter, "unexpected packet 0x{id:02x} in Play state")
            }
            Self::TeleportTimeout => formatter.write_str("teleport confirmation timeout"),
            Self::InvalidPosition => formatter.write_str("invalid position (NaN or Inf)"),
            Self::KeepAliveTimeout => formatter.write_str("keep alive timeout"),
        }
    }
}

impl std::error::Error for SessionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Play(error) => Some(error),
            Self::Framing(error) => Some(error),
            _ => None,
        }
    }
}

impl From<std::io::Error> for SessionError {
    fn from(error: std::io::Error) -> Self {
        if error.kind() == std::io::ErrorKind::UnexpectedEof {
            Self::Disconnected
        } else {
            Self::Io(error)
        }
    }
}

impl From<PlayError> for SessionError {
    fn from(error: PlayError) -> Self {
        Self::Play(error)
    }
}

impl From<rustbound_protocol::framing::FramingError> for SessionError {
    fn from(error: rustbound_protocol::framing::FramingError) -> Self {
        Self::Framing(error)
    }
}

/// Configuration for creating a player session.
#[derive(Debug, Clone)]
pub struct SessionConfig {
    /// The player's UUID.
    pub uuid: Uuid,
    /// The player's username.
    pub username: String,
    /// The player's gamemode (0=Survival, 1=Creative).
    pub gamemode: u8,
    /// The maximum frame length for encoding.
    pub max_frame_length: usize,
    /// The read timeout for the TCP stream.
    pub read_timeout: Duration,
    /// Compression threshold (-1 = disabled, >= 0 = enabled).
    pub compression_threshold: i32,
    /// The server view distance (in chunks).
    pub view_distance: i32,
    /// The server simulation distance (in chunks).
    pub simulation_distance: i32,
    /// The maximum number of players.
    pub max_players: i32,
    /// Keep Alive timeout: clients that don't respond within this duration are kicked.
    pub keep_alive_timeout: Duration,
    /// Enabled hakoniwa dimensions (Join Game list).
    pub enabled_dimensions: crate::hakoniwa::DimensionSet,
}

/// A player session in the Play state.
///
/// Created after Login Success, this struct manages the Join Game handshake,
/// KeepAlive cycling, and position updates for a single player connection.
pub struct PlayerSession {
    /// The player's entity ID.
    entity_id: i32,
    /// The player's UUID.
    uuid: Uuid,
    /// The player's username.
    username: String,
    /// The player's gamemode.
    gamemode: GameMode,
    /// The next teleport ID to assign.
    next_teleport_id: i32,
    /// Sender to the tick loop for state messages.
    tick_sender: Sender<TickMessage>,
    /// Receiver for events from the tick loop.
    event_receiver: Receiver<SessionEvent>,
    /// The maximum frame length for encoding.
    max_frame_length: usize,
    /// Compression threshold (-1 = disabled, >= 0 = enabled).
    compression_threshold: i32,
    /// Last known position X (cached for rotation-only updates).
    last_x: f64,
    /// Last known position Y.
    last_y: f64,
    /// Last known position Z.
    last_z: f64,
    /// Last known yaw (degrees).
    last_yaw: f32,
    /// Last known pitch (degrees).
    last_pitch: f32,
    /// The server view distance (in chunks).
    view_distance: i32,
    /// The server simulation distance (in chunks).
    simulation_distance: i32,
    /// The maximum number of players.
    max_players: i32,
    /// Keep Alive timeout: clients that don't respond within this duration are kicked.
    keep_alive_timeout: Duration,
    /// The payload of the last KeepAlive sent to the client (0 = none pending).
    last_keep_alive_payload: i64,
    /// The instant the last KeepAlive was sent (None = no pending KeepAlive).
    last_keep_alive_sent: Option<std::time::Instant>,
    /// The instant the client last responded to a KeepAlive.
    last_keep_alive_response: std::time::Instant,
    /// Current dimension (drives Chunk Data terrain fill).
    dimension: crate::hakoniwa::DimensionId,
    /// Enabled dimensions advertised in Join Game.
    enabled_dimensions: crate::hakoniwa::DimensionSet,
}

impl PlayerSession {
    /// Creates a new player session, registering with the tick loop.
    ///
    /// This sends a `PlayerJoined` message to the tick loop and creates
    /// a channel for receiving session events.
    pub fn new(
        config: &SessionConfig,
        entity_id: i32,
        tick_sender: Sender<TickMessage>,
    ) -> Result<Self, SessionError> {
        let (event_tx, event_rx) = channel::<SessionEvent>();

        let gamemode = GameMode::from_wire(config.gamemode).ok_or(SessionError::Play(
            PlayError::Codec(rustbound_protocol::primitives::CodecError::InvalidBoolean),
        ))?;

        // Register with the tick loop
        tick_sender
            .send(TickMessage::PlayerJoined {
                entity_id,
                uuid: config.uuid,
                username: config.username.clone(),
                gamemode: config.gamemode,
                view_distance: config.view_distance,
                event_sender: event_tx,
            })
            .map_err(|_| SessionError::Disconnected)?;

        Ok(Self {
            entity_id,
            uuid: config.uuid,
            username: config.username.clone(),
            gamemode,
            next_teleport_id: 0,
            tick_sender,
            event_receiver: event_rx,
            max_frame_length: config.max_frame_length,
            compression_threshold: config.compression_threshold,
            last_x: 0.0,
            last_y: 64.0,
            last_z: 0.0,
            last_yaw: 0.0,
            last_pitch: 0.0,
            view_distance: config.view_distance,
            simulation_distance: config.simulation_distance,
            max_players: config.max_players,
            keep_alive_timeout: config.keep_alive_timeout,
            last_keep_alive_payload: 0,
            last_keep_alive_sent: None,
            last_keep_alive_response: std::time::Instant::now(),
            dimension: crate::hakoniwa::DimensionId::Overworld,
            enabled_dimensions: config.enabled_dimensions,
        })
    }

    /// Returns the player's entity ID.
    pub fn entity_id(&self) -> i32 {
        self.entity_id
    }

    /// Returns the player's UUID.
    pub fn uuid(&self) -> Uuid {
        self.uuid
    }

    /// Returns the player's username.
    pub fn username(&self) -> &str {
        &self.username
    }

    /// Sends an already-encoded packet wire, re-encoding with compression
    /// if a compression threshold has been negotiated.
    ///
    /// The input `wire` must be a single uncompressed frame produced by one
    /// of the `encode_*` functions (i.e. `[length][packet_id][payload]`).
    fn send_wire(&self, stream: &mut TcpStream, wire: &[u8]) -> Result<(), SessionError> {
        if self.compression_threshold < 0 {
            // Compression disabled: send as-is
            stream.write_all(wire)?;
            return Ok(());
        }
        // Compression enabled: extract packet_id + payload from the
        // uncompressed frame, then re-encode as a compressed frame.
        let mut input = wire;
        let frame = rustbound_protocol::framing::decode_frame(&mut input, self.max_frame_length)
            .map_err(|e| SessionError::Play(PlayError::from(e)))?;
        match frame {
            rustbound_protocol::framing::DecodeOutcome::Complete(f) => {
                let mut compressed = Vec::new();
                rustbound_protocol::compression::encode_compressed_frame(
                    f.packet_id,
                    f.payload,
                    self.compression_threshold,
                    self.max_frame_length,
                    &mut compressed,
                )
                .map_err(|e| SessionError::Play(PlayError::from(e)))?;
                stream.write_all(&compressed)?;
            }
            rustbound_protocol::framing::DecodeOutcome::Incomplete => {
                return Err(SessionError::Play(PlayError::Codec(
                    rustbound_protocol::primitives::CodecError::IncompleteInput,
                )));
            }
        }
        Ok(())
    }

    /// Sends the Join Game packet to the client.
    pub fn send_join_game(&self, stream: &mut TcpStream) -> Result<(), SessionError> {
        let registry_codec = minimal_registry_codec();
        eprintln!(
            "sending Join Game to '{}' (entity={}, registry_nbt={} bytes)",
            self.username,
            self.entity_id,
            registry_codec.len()
        );
        let dim = self.dimension.protocol_name().to_string();
        let join_game = JoinGame {
            entity_id: self.entity_id,
            is_hardcore: false,
            gamemode: self.gamemode,
            previous_gamemode: None,
            dimension_names: self.enabled_dimensions.protocol_names(),
            registry_codec,
            dimension_type: dim.clone(),
            dimension_name: dim,
            hashed_seed: 0,
            max_players: self.max_players,
            view_distance: self.view_distance,
            simulation_distance: self.simulation_distance,
            reduce_debug_info: false,
            enable_respawn_screen: true,
            is_debug: false,
            is_flat: true,
            has_death_location: false,
            death_dimension_name: String::new(),
            death_location: (0, 0, 0),
            portal_cooldown: 0,
        };
        let mut wire = Vec::new();
        encode_join_game(&join_game, self.max_frame_length, &mut wire)?;
        self.send_wire(stream, &wire)?;
        eprintln!("Join Game sent to '{}'", self.username);
        Ok(())
    }

    /// Sends the vanilla join-sequence packets after Join Game.
    ///
    /// This sends the minimum set of clientbound Play packets required for a
    /// vanilla 1.20.1 client to proceed past the "Loading terrain" screen
    /// (once chunks are available). The sequence is:
    ///
    /// 1. Plugin Message (brand)
    /// 2. Change Difficulty
    /// 3. Player Abilities (gamemode-aware)
    /// 4. Set Held Item
    /// 5. Entity Event (player status)
    /// 6. Update Recipes (empty stub)
    /// 7. Update Tags (empty registries stub)
    /// 8. Player Info Update (self)
    /// 9. Set Default Spawn Position
    /// 10. Set Center Chunk
    /// 11. Set Render Distance
    /// 12. Set Simulation Distance
    /// 13. Game Event (start waiting for level chunks)
    pub fn send_join_sequence(&self, stream: &mut TcpStream) -> Result<(), SessionError> {
        let mfl = self.max_frame_length;

        // 1. Plugin Message (brand) — 0x17
        let brand = PluginMessageClientbound {
            channel: "minecraft:brand".to_string(),
            data: b"rustbound".to_vec(),
        };
        let mut wire = Vec::new();
        encode_plugin_message_clientbound(&brand, mfl, &mut wire)?;
        self.send_wire(stream, &wire)?;

        // 2. Change Difficulty — 0x0C
        let diff = ChangeDifficulty {
            difficulty: 1, // Easy
            locked: false,
        };
        wire.clear();
        encode_change_difficulty(&diff, mfl, &mut wire)?;
        self.send_wire(stream, &wire)?;

        // 3. Player Abilities — 0x34 (flags depend on gamemode)
        let abilities = PlayerAbilities {
            flags: player_abilities_flags(self.gamemode),
            flying_speed: 0.05,
            fov_modifier: 0.1,
        };
        wire.clear();
        encode_player_abilities(&abilities, mfl, &mut wire)?;
        self.send_wire(stream, &wire)?;

        // 4. Set Held Item — 0x4D
        let held = SetHeldItem { slot: 0 };
        wire.clear();
        encode_set_held_item(&held, mfl, &mut wire)?;
        self.send_wire(stream, &wire)?;

        // 5. Entity Event (player status: 28 = op permission level 4) — 0x1C
        let entity_event = EntityEvent {
            entity_id: self.entity_id,
            entity_status: 28,
        };
        wire.clear();
        encode_entity_event(&entity_event, mfl, &mut wire)?;
        self.send_wire(stream, &wire)?;

        // 6. Update Recipes (empty) — 0x6D
        wire.clear();
        rustbound_protocol::play::encode_update_recipes_empty(mfl, &mut wire)?;
        self.send_wire(stream, &wire)?;

        // 7. Update Tags (empty registries) — 0x6E
        wire.clear();
        rustbound_protocol::play::encode_update_tags_empty(mfl, &mut wire)?;
        self.send_wire(stream, &wire)?;

        // 8. Player Info Update for self — 0x3A
        self.send_player_info_add(
            stream,
            self.entity_id,
            self.uuid,
            &self.username,
            self.gamemode.to_wire(),
        )?;

        // 9. Set Default Spawn Position — 0x50
        let spawn = SetDefaultSpawnPosition {
            location: (0, 64, 0),
            angle: 0.0,
        };
        wire.clear();
        encode_set_default_spawn_position(&spawn, mfl, &mut wire)?;
        self.send_wire(stream, &wire)?;

        // 10. Set Center Chunk — 0x4E
        let center = SetCenterChunk {
            chunk_x: 0,
            chunk_z: 0,
        };
        wire.clear();
        encode_set_center_chunk(&center, mfl, &mut wire)?;
        self.send_wire(stream, &wire)?;

        // 11. Set Render Distance — 0x4F
        let render = SetRenderDistance {
            view_distance: self.view_distance,
        };
        wire.clear();
        encode_set_render_distance(&render, mfl, &mut wire)?;
        self.send_wire(stream, &wire)?;

        // 12. Set Simulation Distance — 0x5C
        let sim = SetSimulationDistance {
            simulation_distance: self.simulation_distance,
        };
        wire.clear();
        encode_set_simulation_distance(&sim, mfl, &mut wire)?;
        self.send_wire(stream, &wire)?;

        // 13. Game Event (13 = Start waiting for level chunks) — 0x1F
        let game_event = GameEvent {
            event_type: 13,
            value: 0.0,
        };
        wire.clear();
        encode_game_event(&game_event, mfl, &mut wire)?;
        self.send_wire(stream, &wire)?;

        Ok(())
    }

    /// Sends initial chunk columns around spawn to the client.
    ///
    /// Generates and sends Chunk Data packets for all chunks within the
    /// given radius of the spawn position (chunk 0,0).
    pub fn send_initial_chunks(
        &self,
        stream: &mut TcpStream,
        radius: i32,
    ) -> Result<(), SessionError> {
        let mfl = self.max_frame_length;
        let positions = crate::world::World::desired_chunks(0, 0, radius);

        for pos in positions {
            let chunk_data = crate::chunk::build_chunk_data_packet(pos.x, pos.z, self.dimension);
            let mut wire = Vec::new();
            encode_chunk_data(&chunk_data, mfl, &mut wire)?;
            self.send_wire(stream, &wire)?;
        }

        Ok(())
    }

    /// Sends a single Chunk Data packet for the given chunk coordinates.
    pub fn send_single_chunk(
        &self,
        stream: &mut TcpStream,
        chunk_x: i32,
        chunk_z: i32,
    ) -> Result<(), SessionError> {
        let chunk_data = crate::chunk::build_chunk_data_packet(chunk_x, chunk_z, self.dimension);
        let mut wire = Vec::new();
        encode_chunk_data(&chunk_data, self.max_frame_length, &mut wire)?;
        self.send_wire(stream, &wire)?;
        Ok(())
    }

    /// Sends a Set Center Chunk packet with the given coordinates.
    pub fn send_set_center_chunk(
        &self,
        stream: &mut TcpStream,
        chunk_x: i32,
        chunk_z: i32,
    ) -> Result<(), SessionError> {
        let packet = SetCenterChunk { chunk_x, chunk_z };
        let mut wire = Vec::new();
        encode_set_center_chunk(&packet, self.max_frame_length, &mut wire)?;
        self.send_wire(stream, &wire)?;
        Ok(())
    }

    /// Sends a System Chat Message to the client.
    ///
    /// The content should be a JSON chat component string. In offline mode,
    /// all chat (player and system) is sent via this packet.
    pub fn send_system_chat(
        &self,
        stream: &mut TcpStream,
        content: &str,
    ) -> Result<(), SessionError> {
        let packet = SystemChatMessage {
            content: content.to_string(),
            overlay: false,
        };
        let mut wire = Vec::new();
        encode_system_chat_message(&packet, self.max_frame_length, &mut wire)?;
        self.send_wire(stream, &wire)?;
        Ok(())
    }

    /// Sends a Synchronize Player Position packet and returns the teleport ID.
    pub fn send_synchronize_position(
        &mut self,
        stream: &mut TcpStream,
    ) -> Result<i32, SessionError> {
        let teleport_id = self.next_teleport_id;
        self.next_teleport_id += 1;

        let packet = SynchronizePlayerPosition {
            x: 0.0,
            y: 64.0,
            z: 0.0,
            yaw: 0.0,
            pitch: 0.0,
            flags: 0,
            teleport_id,
        };
        let mut wire = Vec::new();
        encode_synchronize_player_position(&packet, self.max_frame_length, &mut wire)?;
        self.send_wire(stream, &wire)?;
        Ok(teleport_id)
    }

    /// Sends a KeepAlive packet to the client and records the send time
    /// for timeout enforcement.
    pub fn send_keep_alive(
        &mut self,
        stream: &mut TcpStream,
        payload: i64,
    ) -> Result<(), SessionError> {
        let packet = KeepAlive { payload };
        let mut wire = Vec::new();
        encode_keep_alive_clientbound(&packet, self.max_frame_length, &mut wire)?;
        self.send_wire(stream, &wire)?;
        self.last_keep_alive_payload = payload;
        self.last_keep_alive_sent = Some(std::time::Instant::now());
        Ok(())
    }

    /// Returns `true` if the client has not responded to the last KeepAlive
    /// within the configured timeout. Returns `false` if no KeepAlive is
    /// pending or the client responded in time.
    pub fn is_keep_alive_timed_out(&self) -> bool {
        match self.last_keep_alive_sent {
            Some(sent_at) => sent_at.elapsed() > self.keep_alive_timeout,
            None => false,
        }
    }

    /// Sends a Player Info Update (add player) for a remote player.
    pub fn send_player_info_add(
        &self,
        stream: &mut TcpStream,
        entity_id: i32,
        uuid: Uuid,
        username: &str,
        gamemode: u8,
    ) -> Result<(), SessionError> {
        let _ = entity_id;
        let packet = rustbound_protocol::play::PlayerInfoUpdate {
            actions: rustbound_protocol::play::PlayerInfoActions::new(
                rustbound_protocol::play::PlayerInfoActions::ADD_PLAYER
                    | rustbound_protocol::play::PlayerInfoActions::UPDATE_GAMEMODE
                    | rustbound_protocol::play::PlayerInfoActions::UPDATE_LISTED
                    | rustbound_protocol::play::PlayerInfoActions::UPDATE_LATENCY,
            ),
            entries: vec![rustbound_protocol::play::PlayerInfoEntry {
                uuid,
                name: username.to_string(),
                properties: vec![],
                gamemode: gamemode as i32,
                listed: true,
                latency: 0,
                display_name: None,
            }],
        };
        let mut wire = Vec::new();
        rustbound_protocol::play::encode_player_info_update(
            &packet,
            self.max_frame_length,
            &mut wire,
        )?;
        self.send_wire(stream, &wire)?;
        Ok(())
    }

    /// Sends a Player Info Remove for a player that left.
    pub fn send_player_info_remove(
        &self,
        stream: &mut TcpStream,
        uuid: Uuid,
    ) -> Result<(), SessionError> {
        let packet = rustbound_protocol::play::PlayerInfoRemove { uuids: vec![uuid] };
        let mut wire = Vec::new();
        rustbound_protocol::play::encode_player_info_remove(
            &packet,
            self.max_frame_length,
            &mut wire,
        )?;
        self.send_wire(stream, &wire)?;
        Ok(())
    }

    /// Sends a Spawn Player packet for a remote player.
    pub fn send_spawn_player(
        &self,
        stream: &mut TcpStream,
        entity_id: i32,
        uuid: Uuid,
        x: f64,
        y: f64,
        z: f64,
    ) -> Result<(), SessionError> {
        let packet = rustbound_protocol::play::SpawnPlayer {
            entity_id,
            uuid,
            x,
            y,
            z,
            yaw: 0,
            pitch: 0,
        };
        let mut wire = Vec::new();
        rustbound_protocol::play::encode_spawn_player(&packet, self.max_frame_length, &mut wire)?;
        self.send_wire(stream, &wire)?;
        Ok(())
    }

    /// Sends a Remove Entities packet for a player that left.
    pub fn send_remove_entities(
        &self,
        stream: &mut TcpStream,
        entity_id: i32,
    ) -> Result<(), SessionError> {
        let packet = rustbound_protocol::play::RemoveEntities {
            entity_ids: vec![entity_id],
        };
        let mut wire = Vec::new();
        rustbound_protocol::play::encode_remove_entities(
            &packet,
            self.max_frame_length,
            &mut wire,
        )?;
        self.send_wire(stream, &wire)?;
        Ok(())
    }

    /// Polls for tick loop events and handles them.
    ///
    /// Returns `true` if at least one event was processed.
    pub fn poll_events(&mut self, stream: &mut TcpStream) -> Result<bool, SessionError> {
        let mut processed = false;
        while let Ok(event) = self.event_receiver.try_recv() {
            match event {
                SessionEvent::KeepAlive { payload } => {
                    self.send_keep_alive(stream, payload)?;
                    processed = true;
                }
                SessionEvent::PlayerJoined {
                    entity_id,
                    uuid,
                    username,
                    gamemode,
                    x,
                    y,
                    z,
                } => {
                    self.send_player_info_add(stream, entity_id, uuid, &username, gamemode)?;
                    self.send_spawn_player(stream, entity_id, uuid, x, y, z)?;
                    processed = true;
                }
                SessionEvent::PlayerLeft { entity_id, uuid } => {
                    self.send_player_info_remove(stream, uuid)?;
                    self.send_remove_entities(stream, entity_id)?;
                    processed = true;
                }
                SessionEvent::BlockUpdate {
                    position,
                    block_state,
                } => {
                    self.send_block_update(stream, position, block_state)?;
                    processed = true;
                }
                SessionEvent::ChunkBlockOverrides { overrides } => {
                    for (position, block_state) in overrides {
                        self.send_block_update(stream, position, block_state)?;
                    }
                    processed = true;
                }
                SessionEvent::EntityMovement {
                    entity_id,
                    old_x,
                    old_y,
                    old_z,
                    new_x,
                    new_y,
                    new_z,
                    new_yaw,
                    new_pitch,
                    on_ground,
                } => {
                    let movement = EntityMovementData {
                        entity_id,
                        old_x,
                        old_y,
                        old_z,
                        new_x,
                        new_y,
                        new_z,
                        new_yaw,
                        new_pitch,
                        on_ground,
                    };
                    self.send_entity_movement(stream, &movement)?;
                    processed = true;
                }
                SessionEvent::SetCenterChunkEvent { chunk_x, chunk_z } => {
                    self.send_set_center_chunk(stream, chunk_x, chunk_z)?;
                    processed = true;
                }
                SessionEvent::LoadChunk { chunk_x, chunk_z } => {
                    self.send_single_chunk(stream, chunk_x, chunk_z)?;
                    processed = true;
                }
                SessionEvent::UnloadChunk { chunk_x, chunk_z } => {
                    let packet = UnloadChunk { chunk_x, chunk_z };
                    let mut wire = Vec::new();
                    encode_unload_chunk(&packet, self.max_frame_length, &mut wire)?;
                    self.send_wire(stream, &wire)?;
                    processed = true;
                }
                SessionEvent::CombatDeath { player_id, message } => {
                    let packet = CombatDeath { player_id, message };
                    let mut wire = Vec::new();
                    encode_combat_death(&packet, self.max_frame_length, &mut wire)?;
                    self.send_wire(stream, &wire)?;
                    processed = true;
                }
                SessionEvent::SetBlockDestroyStage {
                    entity_id,
                    position,
                    destroy_stage,
                } => {
                    let packet = SetBlockDestroyStage {
                        entity_id,
                        position,
                        destroy_stage,
                    };
                    let mut wire = Vec::new();
                    encode_set_block_destroy_stage(&packet, self.max_frame_length, &mut wire)?;
                    self.send_wire(stream, &wire)?;
                    processed = true;
                }
                SessionEvent::SetContainerContent {
                    window_id,
                    state_id,
                    slots,
                    carried_item,
                } => {
                    self.send_set_container_content(
                        stream,
                        window_id,
                        state_id,
                        &slots,
                        &carried_item,
                    )?;
                    processed = true;
                }
                SessionEvent::SetContainerSlot {
                    window_id,
                    state_id,
                    slot,
                    item,
                } => {
                    self.send_set_container_slot(stream, window_id, state_id, slot, &item)?;
                    processed = true;
                }
                SessionEvent::OpenScreen {
                    window_id,
                    window_type,
                    title,
                } => {
                    let packet = rustbound_protocol::play::OpenScreen {
                        window_id,
                        window_type,
                        title,
                    };
                    let mut wire = Vec::new();
                    rustbound_protocol::play::encode_open_screen(
                        &packet,
                        self.max_frame_length,
                        &mut wire,
                    )?;
                    self.send_wire(stream, &wire)?;
                    processed = true;
                }
                SessionEvent::CloseContainer { window_id } => {
                    let packet = rustbound_protocol::play::CloseContainer { window_id };
                    let mut wire = Vec::new();
                    rustbound_protocol::play::encode_close_container_clientbound(
                        &packet,
                        self.max_frame_length,
                        &mut wire,
                    )?;
                    self.send_wire(stream, &wire)?;
                    processed = true;
                }
                SessionEvent::SetHeldItemClientbound { slot } => {
                    let held = SetHeldItem { slot };
                    let mut wire = Vec::new();
                    encode_set_held_item(&held, self.max_frame_length, &mut wire)?;
                    self.send_wire(stream, &wire)?;
                    processed = true;
                }
                SessionEvent::SystemChat { content } => {
                    self.send_system_chat(stream, &content)?;
                    processed = true;
                }
                SessionEvent::SetHealth {
                    health,
                    food,
                    food_saturation,
                } => {
                    self.send_set_health(stream, health, food, food_saturation)?;
                    processed = true;
                }
                SessionEvent::RespawnPlayer {
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
                    x,
                    y,
                    z,
                } => {
                    if let Some(dim) = crate::hakoniwa::DimensionId::parse_protocol(&dimension_name)
                    {
                        self.dimension = dim;
                    }
                    self.send_respawn(
                        stream,
                        &dimension_type,
                        &dimension_name,
                        hashed_seed,
                        gamemode,
                        previous_gamemode,
                        is_debug,
                        is_flat,
                        has_death_location,
                        &death_dimension_name,
                        death_location,
                        portal_cooldown,
                        data_kept,
                    )?;
                    // After respawn, re-synchronize position to spawn point
                    self.last_x = x;
                    self.last_y = y;
                    self.last_z = z;
                    let teleport_id = self.next_teleport_id;
                    self.next_teleport_id += 1;
                    let sync = SynchronizePlayerPosition {
                        x,
                        y,
                        z,
                        yaw: 0.0,
                        pitch: 0.0,
                        flags: 0,
                        teleport_id,
                    };
                    let mut wire = Vec::new();
                    encode_synchronize_player_position(&sync, self.max_frame_length, &mut wire)?;
                    self.send_wire(stream, &wire)?;
                    processed = true;
                }
                SessionEvent::SynchronizePosition { x, y, z } => {
                    self.last_x = x;
                    self.last_y = y;
                    self.last_z = z;
                    let teleport_id = self.next_teleport_id;
                    self.next_teleport_id += 1;
                    // Absolute XYZ, but keep the client's look direction.
                    // flags bit3=yaw relative, bit4=pitch relative (delta 0 ⇒ unchanged).
                    // Resetting yaw/pitch to 0 every correction locks the camera and
                    // can push vanilla into the "loading terrain" screen.
                    let sync = SynchronizePlayerPosition {
                        x,
                        y,
                        z,
                        yaw: 0.0,
                        pitch: 0.0,
                        flags: 0x18,
                        teleport_id,
                    };
                    let mut wire = Vec::new();
                    encode_synchronize_player_position(&sync, self.max_frame_length, &mut wire)?;
                    self.send_wire(stream, &wire)?;
                    processed = true;
                }
                SessionEvent::SpawnMob {
                    entity_id,
                    uuid,
                    entity_type,
                    x,
                    y,
                    z,
                    yaw,
                    pitch,
                } => {
                    let yaw_b = degrees_to_angle_byte(yaw);
                    let pitch_b = degrees_to_angle_byte(pitch);
                    let packet = rustbound_protocol::play::SpawnEntity {
                        entity_id,
                        uuid,
                        entity_type,
                        x,
                        y,
                        z,
                        pitch: pitch_b,
                        yaw: yaw_b,
                        head_yaw: yaw_b,
                        data: 0,
                        velocity_x: 0,
                        velocity_y: 0,
                        velocity_z: 0,
                    };
                    let mut wire = Vec::new();
                    rustbound_protocol::play::encode_spawn_entity(
                        &packet,
                        self.max_frame_length,
                        &mut wire,
                    )?;
                    self.send_wire(stream, &wire)?;
                    processed = true;
                }
                SessionEvent::RemoveEntities { entity_ids } => {
                    let packet = rustbound_protocol::play::RemoveEntities { entity_ids };
                    let mut wire = Vec::new();
                    rustbound_protocol::play::encode_remove_entities(
                        &packet,
                        self.max_frame_length,
                        &mut wire,
                    )?;
                    self.send_wire(stream, &wire)?;
                    processed = true;
                }
            }
        }
        Ok(processed)
    }

    /// Handles a decoded play packet from the client.
    pub fn handle_play_packet(&mut self, packet: PlayPacket) -> Result<Option<i32>, SessionError> {
        match packet {
            PlayPacket::ConfirmTeleportation(ConfirmTeleportation { teleport_id: _ }) => {
                // Teleport confirmed - no action needed for now
                Ok(None)
            }
            PlayPacket::KeepAliveServerbound(KeepAlive { payload }) => {
                // KeepAlive response received - clear the pending timer.
                // Only accept the response if it matches the last sent payload.
                if self.last_keep_alive_sent.is_some() && payload == self.last_keep_alive_payload {
                    self.last_keep_alive_sent = None;
                    self.last_keep_alive_response = std::time::Instant::now();
                }
                Ok(None)
            }
            PlayPacket::ClientInformation(ClientInformation { view_distance, .. }) => {
                // Forward the client's view distance to the tick loop so it
                // can adjust the chunk loading set.
                let _ = self.tick_sender.send(TickMessage::SetClientViewDistance {
                    entity_id: self.entity_id,
                    view_distance: view_distance as i32,
                });
                Ok(None)
            }
            PlayPacket::SetPlayerPosition(pos) => {
                if !is_finite_position(pos.x, pos.y, pos.z) {
                    return Err(SessionError::InvalidPosition);
                }
                self.last_x = pos.x;
                self.last_y = pos.y;
                self.last_z = pos.z;
                let _ = self.tick_sender.send(TickMessage::PlayerPositionUpdate {
                    entity_id: self.entity_id,
                    x: pos.x,
                    y: pos.y,
                    z: pos.z,
                    yaw: self.last_yaw,
                    pitch: self.last_pitch,
                    on_ground: pos.on_ground,
                });
                Ok(None)
            }
            PlayPacket::SetPlayerPositionAndRotation(pos) => {
                if !is_finite_position(pos.x, pos.y, pos.z)
                    || !pos.yaw.is_finite()
                    || !pos.pitch.is_finite()
                {
                    return Err(SessionError::InvalidPosition);
                }
                self.last_x = pos.x;
                self.last_y = pos.y;
                self.last_z = pos.z;
                self.last_yaw = pos.yaw;
                self.last_pitch = pos.pitch;
                let _ = self.tick_sender.send(TickMessage::PlayerPositionUpdate {
                    entity_id: self.entity_id,
                    x: pos.x,
                    y: pos.y,
                    z: pos.z,
                    yaw: pos.yaw,
                    pitch: pos.pitch,
                    on_ground: pos.on_ground,
                });
                Ok(None)
            }
            PlayPacket::SetPlayerRotation(rot) => {
                if !rot.yaw.is_finite() || !rot.pitch.is_finite() {
                    return Err(SessionError::InvalidPosition);
                }
                self.last_yaw = rot.yaw;
                self.last_pitch = rot.pitch;
                let _ = self.tick_sender.send(TickMessage::PlayerPositionUpdate {
                    entity_id: self.entity_id,
                    x: self.last_x,
                    y: self.last_y,
                    z: self.last_z,
                    yaw: rot.yaw,
                    pitch: rot.pitch,
                    on_ground: rot.on_ground,
                });
                Ok(None)
            }
            PlayPacket::PlayerDigging(dig) => {
                // Creative mode: instant break on StartDestroy
                // Survival: forward to tick loop for break progress tracking
                if self.gamemode == GameMode::Creative
                    && matches!(
                        dig.action,
                        rustbound_protocol::play::PlayerDiggingAction::StartDestroy
                    )
                {
                    // Set block to air (state 0)
                    let _ = self.tick_sender.send(TickMessage::SetBlock {
                        entity_id: Some(self.entity_id),
                        position: dig.position,
                        block_state: 0,
                    });
                } else if self.gamemode != GameMode::Creative
                    && matches!(
                        dig.action,
                        rustbound_protocol::play::PlayerDiggingAction::StartDestroy
                            | rustbound_protocol::play::PlayerDiggingAction::AbortDestroy
                            | rustbound_protocol::play::PlayerDiggingAction::StopDestroy
                    )
                {
                    // Forward dig action to tick loop for break progress
                    let action = match dig.action {
                        rustbound_protocol::play::PlayerDiggingAction::StartDestroy => 0,
                        rustbound_protocol::play::PlayerDiggingAction::AbortDestroy => 1,
                        rustbound_protocol::play::PlayerDiggingAction::StopDestroy => 2,
                        _ => 0,
                    };
                    let _ = self.tick_sender.send(TickMessage::DigBlock {
                        entity_id: self.entity_id,
                        action,
                        position: dig.position,
                    });
                }
                // Always ACK the dig packet (protocol requires it)
                Ok(Some(dig.sequence))
            }
            PlayPacket::UseItemOn(place) => {
                // Always forward: tick opens chests in any gamemode, and places
                // blocks when Creative is holding a block item.
                let _ = self.tick_sender.send(TickMessage::PlaceBlock {
                    entity_id: self.entity_id,
                    position: place.position,
                    face: place.face as i32,
                });
                // Always ACK the place packet (protocol requires it)
                Ok(Some(place.sequence))
            }
            PlayPacket::ClickContainer(click) => {
                let _ = self.tick_sender.send(TickMessage::ClickContainer {
                    entity_id: self.entity_id,
                    window_id: click.window_id,
                    state_id: click.state_id,
                    changed_slots: click.changed_slots,
                    cursor: click.cursor,
                });
                Ok(None)
            }
            PlayPacket::CloseContainerServerbound(close) => {
                let _ = self.tick_sender.send(TickMessage::CloseContainer {
                    entity_id: self.entity_id,
                    window_id: close.window_id,
                });
                Ok(None)
            }
            PlayPacket::SetCreativeModeSlot(creative) => {
                // Forward to tick loop for inventory tracking
                let _ = self.tick_sender.send(TickMessage::SetCreativeSlot {
                    entity_id: self.entity_id,
                    slot: creative.slot,
                    item: creative.item,
                });
                Ok(None)
            }
            PlayPacket::SetHeldItemServerbound(held) => {
                // Forward to tick loop for held slot tracking
                let _ = self.tick_sender.send(TickMessage::SetHeldItem {
                    entity_id: self.entity_id,
                    slot: held.slot,
                });
                Ok(None)
            }
            PlayPacket::ChatMessageServerbound(chat) => {
                // Forward chat to tick loop for broadcasting
                let _ = self.tick_sender.send(TickMessage::ChatMessage {
                    entity_id: self.entity_id,
                    uuid: self.uuid,
                    username: self.username.clone(),
                    message: chat.message,
                });
                Ok(None)
            }
            PlayPacket::ClientStatus(status) => {
                // Forward to tick loop for respawn handling
                let _ = self.tick_sender.send(TickMessage::ClientStatus {
                    entity_id: self.entity_id,
                    action: status.action,
                });
                Ok(None)
            }
            PlayPacket::InteractEntity(interact) => {
                let _ = self.tick_sender.send(TickMessage::InteractEntity {
                    entity_id: self.entity_id,
                    target: interact.entity_id,
                    action: interact.action,
                });
                Ok(None)
            }
            _ => Err(SessionError::UnexpectedPacket(-1)),
        }
    }

    /// Sends a Block Update packet to the client.
    pub fn send_block_update(
        &self,
        stream: &mut TcpStream,
        position: (i32, i32, i32),
        block_state: i32,
    ) -> Result<(), SessionError> {
        let packet = rustbound_protocol::play::BlockUpdate {
            position,
            block_state,
        };
        let mut wire = Vec::new();
        rustbound_protocol::play::encode_block_update(&packet, self.max_frame_length, &mut wire)?;
        self.send_wire(stream, &wire)?;
        Ok(())
    }

    /// Sends a Set Container Content packet to the client.
    ///
    /// Used to initialize or fully sync the player's inventory.
    pub fn send_set_container_content(
        &self,
        stream: &mut TcpStream,
        window_id: u8,
        state_id: i32,
        slots: &[rustbound_protocol::play::Slot],
        carried_item: &rustbound_protocol::play::Slot,
    ) -> Result<(), SessionError> {
        let packet = SetContainerContent {
            window_id,
            state_id,
            slots: slots.to_vec(),
            carried_item: carried_item.clone(),
        };
        let mut wire = Vec::new();
        encode_set_container_content(&packet, self.max_frame_length, &mut wire)?;
        self.send_wire(stream, &wire)?;
        Ok(())
    }

    /// Sends a Set Container Slot packet to the client.
    ///
    /// Used to update a single inventory slot.
    pub fn send_set_container_slot(
        &self,
        stream: &mut TcpStream,
        window_id: i8,
        state_id: i32,
        slot: i16,
        item: &rustbound_protocol::play::Slot,
    ) -> Result<(), SessionError> {
        let packet = SetContainerSlot {
            window_id,
            state_id,
            slot,
            item: item.clone(),
        };
        let mut wire = Vec::new();
        encode_set_container_slot(&packet, self.max_frame_length, &mut wire)?;
        self.send_wire(stream, &wire)?;
        Ok(())
    }

    /// Sends a Set Health packet to the client.
    pub fn send_set_health(
        &self,
        stream: &mut TcpStream,
        health: f32,
        food: i32,
        food_saturation: f32,
    ) -> Result<(), SessionError> {
        let packet = SetHealth {
            health,
            food,
            food_saturation,
        };
        let mut wire = Vec::new();
        encode_set_health(&packet, self.max_frame_length, &mut wire)?;
        self.send_wire(stream, &wire)?;
        Ok(())
    }

    /// Sends a Respawn packet to the client.
    #[allow(clippy::too_many_arguments)]
    pub fn send_respawn(
        &self,
        stream: &mut TcpStream,
        dimension_type: &str,
        dimension_name: &str,
        hashed_seed: i64,
        gamemode: u8,
        previous_gamemode: i8,
        is_debug: bool,
        is_flat: bool,
        has_death_location: bool,
        death_dimension_name: &str,
        death_location: (i32, i32, i32),
        portal_cooldown: i32,
        data_kept: u8,
    ) -> Result<(), SessionError> {
        let packet = Respawn {
            dimension_type: dimension_type.to_string(),
            dimension_name: dimension_name.to_string(),
            hashed_seed,
            gamemode,
            previous_gamemode,
            is_debug,
            is_flat,
            has_death_location,
            death_dimension_name: death_dimension_name.to_string(),
            death_location,
            portal_cooldown,
            data_kept,
        };
        let mut wire = Vec::new();
        encode_respawn(&packet, self.max_frame_length, &mut wire)?;
        self.send_wire(stream, &wire)?;
        Ok(())
    }

    /// Sends an Acknowledge Block Change packet to the client.
    ///
    /// This confirms that the server has processed a block change initiated
    /// by the client (dig/place). The sequence number must match the one
    /// from the client's Player Digging or Use Item On packet.
    pub fn send_acknowledge_block_change(
        &self,
        stream: &mut TcpStream,
        sequence: i32,
    ) -> Result<(), SessionError> {
        let packet = rustbound_protocol::play::AcknowledgeBlockChange { sequence };
        let mut wire = Vec::new();
        rustbound_protocol::play::encode_acknowledge_block_change(
            &packet,
            self.max_frame_length,
            &mut wire,
        )?;
        self.send_wire(stream, &wire)?;
        Ok(())
    }

    /// Sends an entity movement packet to the client.
    ///
    /// Chooses the appropriate packet type based on the position/rotation delta:
    /// - No change: nothing is sent
    /// - Position only (small delta): Move Entity (Pos) `0x29`
    /// - Rotation only: Move Entity (Rot) `0x2B`
    /// - Both (small delta): Move Entity (Pos+Rot) `0x2A`
    /// - Large position delta (>8 blocks): Entity Teleport `0x68`
    ///
    /// The delta values for relative moves are in 1/4096 of a block.
    /// Angles are converted from degrees to byte steps (256 = 360 degrees).
    pub fn send_entity_movement(
        &self,
        stream: &mut TcpStream,
        movement: &EntityMovementData,
    ) -> Result<(), SessionError> {
        let dx = movement.new_x - movement.old_x;
        let dy = movement.new_y - movement.old_y;
        let dz = movement.new_z - movement.old_z;
        let pos_changed = dx != 0.0 || dy != 0.0 || dz != 0.0;
        // We always send rotation since we don't track old rotation per-entity
        // in the session. The tick loop only sends EntityMovement when something
        // changed (position or rotation).
        let yaw_byte = degrees_to_angle_byte(movement.new_yaw);
        let pitch_byte = degrees_to_angle_byte(movement.new_pitch);

        if !pos_changed {
            // Rotation only
            let packet = rustbound_protocol::play::MoveEntityRot {
                entity_id: movement.entity_id,
                yaw: yaw_byte,
                pitch: pitch_byte,
                on_ground: movement.on_ground,
            };
            let mut wire = Vec::new();
            rustbound_protocol::play::encode_move_entity_rot(
                &packet,
                self.max_frame_length,
                &mut wire,
            )?;
            self.send_wire(stream, &wire)?;
            return Ok(());
        }

        // Check if delta fits in i16 (max ~8 blocks at 1/4096 resolution)
        const DELTA_SCALE: f64 = 4096.0;
        const MAX_DELTA: f64 = 32767.0 / DELTA_SCALE; // ~7.999 blocks
        let abs_dx = dx.abs();
        let abs_dy = dy.abs();
        let abs_dz = dz.abs();
        if abs_dx > MAX_DELTA || abs_dy > MAX_DELTA || abs_dz > MAX_DELTA {
            // Use Entity Teleport (absolute position)
            let packet = rustbound_protocol::play::EntityTeleport {
                entity_id: movement.entity_id,
                x: movement.new_x,
                y: movement.new_y,
                z: movement.new_z,
                yaw: yaw_byte,
                pitch: pitch_byte,
                on_ground: movement.on_ground,
            };
            let mut wire = Vec::new();
            rustbound_protocol::play::encode_entity_teleport(
                &packet,
                self.max_frame_length,
                &mut wire,
            )?;
            self.send_wire(stream, &wire)?;
        } else {
            // Relative move
            let delta_x = (dx * DELTA_SCALE).round() as i16;
            let delta_y = (dy * DELTA_SCALE).round() as i16;
            let delta_z = (dz * DELTA_SCALE).round() as i16;
            let packet = rustbound_protocol::play::MoveEntityPosRot {
                entity_id: movement.entity_id,
                delta_x,
                delta_y,
                delta_z,
                yaw: yaw_byte,
                pitch: pitch_byte,
                on_ground: movement.on_ground,
            };
            let mut wire = Vec::new();
            rustbound_protocol::play::encode_move_entity_pos_rot(
                &packet,
                self.max_frame_length,
                &mut wire,
            )?;
            self.send_wire(stream, &wire)?;
        }
        Ok(())
    }

    /// Sends a Disconnect (Play) packet with a reason string.
    ///
    /// Best-effort: ignores I/O errors since the connection may already be
    /// broken.
    pub fn send_disconnect(&self, stream: &mut TcpStream, reason: &str) {
        let packet = DisconnectPlay {
            reason: format!(r#"{{"text":"{reason}"}}"#),
        };
        let mut wire = Vec::new();
        if encode_disconnect_play(&packet, self.max_frame_length, &mut wire).is_ok() {
            let _ = stream.write_all(&wire);
        }
    }

    /// Notifies the tick loop that this player has left.
    pub fn shutdown(&self) {
        let _ = self.tick_sender.send(TickMessage::PlayerLeft {
            entity_id: self.entity_id,
        });
    }
}

impl Drop for PlayerSession {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Allocates entity IDs for player sessions.
///
/// This is a simple atomic counter shared between connection handlers.
/// It provides a narrow, lock-free synchronization boundary.
#[derive(Debug)]
pub struct EntityIdAllocator {
    counter: Arc<AtomicI32>,
}

impl EntityIdAllocator {
    /// Creates a new allocator starting at the given ID.
    pub fn new(start: i32) -> Self {
        Self {
            counter: Arc::new(AtomicI32::new(start)),
        }
    }

    /// Allocates the next entity ID.
    pub fn allocate(&self) -> i32 {
        self.counter.fetch_add(1, Ordering::Relaxed)
    }

    /// Creates a clone that shares the same counter.
    pub fn shared_clone(&self) -> Self {
        Self {
            counter: self.counter.clone(),
        }
    }
}

impl Default for EntityIdAllocator {
    fn default() -> Self {
        Self::new(1)
    }
}

impl Clone for EntityIdAllocator {
    fn clone(&self) -> Self {
        self.shared_clone()
    }
}

/// Provides a minimal registry codec NBT blob.
///
/// This is a valid NBT compound tag with an empty name and no entries:
/// `0x0a` (TAG_Compound) + empty name (2 zero bytes) + `0x00` (TAG_End).
/// Returns true if all three coordinates are finite (not NaN or Inf).
fn is_finite_position(x: f64, y: f64, z: f64) -> bool {
    x.is_finite() && y.is_finite() && z.is_finite()
}

fn minimal_registry_codec() -> Vec<u8> {
    rustbound_protocol::registry_codec::build_registry_codec()
}

/// Reads and decodes a single play packet from the buffer.
///
/// Returns `Ok(Some(packet))` if a complete packet was decoded,
/// `Ok(None)` if more data is needed, or `Err` on error.
///
/// When `compression_threshold >= 0`, the input buffer is expected to contain
/// compressed frames. The compressed frame is decoded, re-encoded as an
/// uncompressed frame into a temporary buffer, and then passed to the
/// existing uncompressed decode path.
pub fn try_decode_play_packet(
    buffer: &mut Vec<u8>,
    max_frame_length: usize,
    compression_threshold: i32,
) -> Result<Option<PlayPacket>, SessionError> {
    if compression_threshold < 0 {
        return try_decode_play_packet_inner(buffer, max_frame_length);
    }

    // Compression enabled: decode one compressed frame, re-encode as
    // uncompressed, then delegate to the inner decoder.
    let source = buffer.as_slice();
    let mut input = source;
    let compressed = match rustbound_protocol::compression::decode_compressed_frame(
        &mut input,
        compression_threshold,
        max_frame_length,
    ) {
        Ok(rustbound_protocol::compression::CompressedDecodeOutcome::Complete(pkt)) => pkt,
        Ok(rustbound_protocol::compression::CompressedDecodeOutcome::Incomplete) => {
            return Ok(None);
        }
        Err(e) => return Err(SessionError::Play(PlayError::from(e))),
    };
    let consumed = source.len() - input.len();
    buffer.drain(..consumed);

    // Re-encode as uncompressed frame into a temporary buffer
    let mut temp = Vec::new();
    rustbound_protocol::framing::encode_frame(
        compressed.packet_id,
        &compressed.payload,
        max_frame_length,
        &mut temp,
    )
    .map_err(|e| SessionError::Play(PlayError::from(e)))?;

    try_decode_play_packet_inner(&mut temp, max_frame_length)
}

fn try_decode_play_packet_inner(
    buffer: &mut Vec<u8>,
    max_frame_length: usize,
) -> Result<Option<PlayPacket>, SessionError> {
    let source = buffer.as_slice();
    let mut input = source;

    // Try each known serverbound packet type. We try Confirm Teleportation
    // (0x00) first, then KeepAlive (0x12), then position/rotation packets.
    // The decoders return WrongPacketId for non-matching IDs, so we try
    // each in order.

    // Confirm Teleportation (0x00)
    match decode_confirm_teleportation(&mut input, max_frame_length) {
        Ok(PlayDecodeOutcome::Complete(packet)) => {
            let consumed = source.len() - input.len();
            buffer.drain(..consumed);
            return Ok(Some(packet));
        }
        Ok(PlayDecodeOutcome::Incomplete) => return Ok(None),
        Err(PlayError::WrongPacketId { .. }) => {
            input = source;
        }
        Err(error) => return Err(SessionError::from(error)),
    }

    // Keep Alive serverbound (0x12)
    match decode_keep_alive_serverbound(&mut input, max_frame_length) {
        Ok(PlayDecodeOutcome::Complete(packet)) => {
            let consumed = source.len() - input.len();
            buffer.drain(..consumed);
            return Ok(Some(packet));
        }
        Ok(PlayDecodeOutcome::Incomplete) => return Ok(None),
        Err(PlayError::WrongPacketId { .. }) => {
            input = source;
        }
        Err(error) => return Err(SessionError::from(error)),
    }

    // Client Information (0x08)
    match decode_client_information(&mut input, max_frame_length) {
        Ok(PlayDecodeOutcome::Complete(packet)) => {
            let consumed = source.len() - input.len();
            buffer.drain(..consumed);
            return Ok(Some(packet));
        }
        Ok(PlayDecodeOutcome::Incomplete) => return Ok(None),
        Err(PlayError::WrongPacketId { .. }) => {
            input = source;
        }
        Err(error) => return Err(SessionError::from(error)),
    }

    // Set Player Position (0x14)
    match decode_set_player_position(&mut input, max_frame_length) {
        Ok(PlayDecodeOutcome::Complete(packet)) => {
            let consumed = source.len() - input.len();
            buffer.drain(..consumed);
            return Ok(Some(packet));
        }
        Ok(PlayDecodeOutcome::Incomplete) => return Ok(None),
        Err(PlayError::WrongPacketId { .. }) => {
            input = source;
        }
        Err(error) => return Err(SessionError::from(error)),
    }

    // Set Player Position and Rotation (0x15)
    match decode_set_player_position_and_rotation(&mut input, max_frame_length) {
        Ok(PlayDecodeOutcome::Complete(packet)) => {
            let consumed = source.len() - input.len();
            buffer.drain(..consumed);
            return Ok(Some(packet));
        }
        Ok(PlayDecodeOutcome::Incomplete) => return Ok(None),
        Err(PlayError::WrongPacketId { .. }) => {
            input = source;
        }
        Err(error) => return Err(SessionError::from(error)),
    }

    // Set Player Rotation (0x16)
    match decode_set_player_rotation(&mut input, max_frame_length) {
        Ok(PlayDecodeOutcome::Complete(packet)) => {
            let consumed = source.len() - input.len();
            buffer.drain(..consumed);
            return Ok(Some(packet));
        }
        Ok(PlayDecodeOutcome::Incomplete) => return Ok(None),
        Err(PlayError::WrongPacketId { .. }) => {
            input = source;
        }
        Err(error) => return Err(SessionError::from(error)),
    }

    // Player Digging (0x1D)
    match decode_player_digging(&mut input, max_frame_length) {
        Ok(PlayDecodeOutcome::Complete(packet)) => {
            let consumed = source.len() - input.len();
            buffer.drain(..consumed);
            return Ok(Some(packet));
        }
        Ok(PlayDecodeOutcome::Incomplete) => return Ok(None),
        Err(PlayError::WrongPacketId { .. }) => {
            input = source;
        }
        Err(error) => return Err(SessionError::from(error)),
    }

    // Interact Entity (0x10)
    match rustbound_protocol::play::decode_interact_entity(&mut input, max_frame_length) {
        Ok(PlayDecodeOutcome::Complete(packet)) => {
            let consumed = source.len() - input.len();
            buffer.drain(..consumed);
            return Ok(Some(packet));
        }
        Ok(PlayDecodeOutcome::Incomplete) => return Ok(None),
        Err(PlayError::WrongPacketId { .. }) => {
            input = source;
        }
        Err(error) => return Err(SessionError::from(error)),
    }

    // Use Item On (0x31)
    match decode_use_item_on(&mut input, max_frame_length) {
        Ok(PlayDecodeOutcome::Complete(packet)) => {
            let consumed = source.len() - input.len();
            buffer.drain(..consumed);
            return Ok(Some(packet));
        }
        Ok(PlayDecodeOutcome::Incomplete) => return Ok(None),
        Err(PlayError::WrongPacketId { .. }) => {
            input = source;
        }
        Err(error) => return Err(SessionError::from(error)),
    }

    // Click Container (0x0B)
    match rustbound_protocol::play::decode_click_container(&mut input, max_frame_length) {
        Ok(PlayDecodeOutcome::Complete(packet)) => {
            let consumed = source.len() - input.len();
            buffer.drain(..consumed);
            return Ok(Some(packet));
        }
        Ok(PlayDecodeOutcome::Incomplete) => return Ok(None),
        Err(PlayError::WrongPacketId { .. }) => {
            input = source;
        }
        Err(error) => return Err(SessionError::from(error)),
    }

    // Close Container serverbound (0x0C)
    match rustbound_protocol::play::decode_close_container_serverbound(
        &mut input,
        max_frame_length,
    ) {
        Ok(PlayDecodeOutcome::Complete(packet)) => {
            let consumed = source.len() - input.len();
            buffer.drain(..consumed);
            return Ok(Some(packet));
        }
        Ok(PlayDecodeOutcome::Incomplete) => return Ok(None),
        Err(PlayError::WrongPacketId { .. }) => {
            input = source;
        }
        Err(error) => return Err(SessionError::from(error)),
    }

    // Set Held Item serverbound (0x28)
    match decode_set_held_item_serverbound(&mut input, max_frame_length) {
        Ok(PlayDecodeOutcome::Complete(packet)) => {
            let consumed = source.len() - input.len();
            buffer.drain(..consumed);
            return Ok(Some(packet));
        }
        Ok(PlayDecodeOutcome::Incomplete) => return Ok(None),
        Err(PlayError::WrongPacketId { .. }) => {
            input = source;
        }
        Err(error) => return Err(SessionError::from(error)),
    }

    // Set Creative Mode Slot (0x2B)
    match decode_set_creative_mode_slot(&mut input, max_frame_length) {
        Ok(PlayDecodeOutcome::Complete(packet)) => {
            let consumed = source.len() - input.len();
            buffer.drain(..consumed);
            return Ok(Some(packet));
        }
        Ok(PlayDecodeOutcome::Incomplete) => return Ok(None),
        Err(PlayError::WrongPacketId { .. }) => {
            input = source;
        }
        Err(error) => return Err(SessionError::from(error)),
    }

    // Chat Message serverbound (0x01)
    match decode_chat_message_serverbound(&mut input, max_frame_length) {
        Ok(PlayDecodeOutcome::Complete(packet)) => {
            let consumed = source.len() - input.len();
            buffer.drain(..consumed);
            return Ok(Some(packet));
        }
        Ok(PlayDecodeOutcome::Incomplete) => return Ok(None),
        Err(PlayError::WrongPacketId { .. }) => {
            input = source;
        }
        Err(error) => return Err(SessionError::from(error)),
    }

    // Client Status (0x08)
    match decode_client_status(&mut input, max_frame_length) {
        Ok(PlayDecodeOutcome::Complete(packet)) => {
            let consumed = source.len() - input.len();
            buffer.drain(..consumed);
            return Ok(Some(packet));
        }
        Ok(PlayDecodeOutcome::Incomplete) => return Ok(None),
        Err(PlayError::WrongPacketId { .. }) => {
            input = source;
        }
        Err(error) => return Err(SessionError::from(error)),
    }

    // Unknown packet: skip it gracefully rather than disconnecting.
    // This allows vanilla clients to send packets we don't yet handle
    // (e.g. inventory, chat, swing arm) without being kicked.
    // We consume one frame from the buffer and discard it.
    match rustbound_protocol::framing::decode_frame(&mut input, max_frame_length) {
        Ok(rustbound_protocol::framing::DecodeOutcome::Complete(_frame)) => {
            let consumed = source.len() - input.len();
            buffer.drain(..consumed);
            // Return None to signal "no actionable packet, continue reading"
            Ok(None)
        }
        Ok(rustbound_protocol::framing::DecodeOutcome::Incomplete) => Ok(None),
        Err(error) => Err(SessionError::from(PlayError::from(error))),
    }
}

/// Runs the play loop for a single connection.
///
/// This function:
/// 1. Creates a PlayerSession
/// 2. Sends Join Game
/// 3. Sends Synchronize Player Position
/// 4. Loops reading packets, handling events, and forwarding state
/// 5. Returns when the client disconnects or an error occurs
pub fn run_play_loop(
    stream: &mut TcpStream,
    session_config: &SessionConfig,
    entity_id: i32,
    tick_sender: Sender<TickMessage>,
) -> Result<(), SessionError> {
    // Update the stream's read timeout to the play read timeout
    stream
        .set_read_timeout(Some(session_config.read_timeout))
        .map_err(SessionError::Io)?;
    stream
        .set_write_timeout(Some(session_config.read_timeout))
        .map_err(SessionError::Io)?;

    let mut session = PlayerSession::new(session_config, entity_id, tick_sender)?;

    // Send Join Game
    session.send_join_game(stream).map_err(|e| {
        eprintln!("failed sending Join Game: {e}");
        e
    })?;

    // Send vanilla join-sequence packets (brand, abilities, spawn pos, etc.)
    session.send_join_sequence(stream).map_err(|e| {
        eprintln!("failed sending join sequence: {e}");
        e
    })?;
    eprintln!("join sequence complete for '{}'", session_config.username);

    // Send initial chunk columns (radius 2 for bootstrapping)
    session.send_initial_chunks(stream, 2).map_err(|e| {
        eprintln!("failed sending initial chunks: {e}");
        e
    })?;
    eprintln!("initial chunks sent for '{}'", session_config.username);

    // Send Synchronize Player Position
    session.send_synchronize_position(stream).map_err(|e| {
        eprintln!("failed sending sync position: {e}");
        e
    })?;
    eprintln!("play handshake finished for '{}'", session_config.username);

    // Main play loop
    let mut read_buffer = Vec::with_capacity(4096);
    let result = loop {
        // Poll for tick events (KeepAlive, etc.)
        if let Err(e) = session.poll_events(stream) {
            break Err(e);
        }

        // Check KeepAlive timeout: kick idle clients
        if session.is_keep_alive_timed_out() {
            session.send_disconnect(stream, "Timed out (no KeepAlive response)");
            break Err(SessionError::KeepAliveTimeout);
        }

        // Try to decode a packet
        match try_decode_play_packet(
            &mut read_buffer,
            session_config.max_frame_length,
            session_config.compression_threshold,
        ) {
            Ok(Some(packet)) => {
                match session.handle_play_packet(packet) {
                    Ok(Some(ack_sequence)) => {
                        // Send Acknowledge Block Change for dig/place packets
                        if let Err(e) = session.send_acknowledge_block_change(stream, ack_sequence)
                        {
                            break Err(e);
                        }
                    }
                    Ok(None) => {}
                    Err(e) => {
                        // Send disconnect before returning the error
                        match &e {
                            SessionError::InvalidPosition => {
                                session.send_disconnect(stream, "Invalid position");
                            }
                            SessionError::UnexpectedPacket(_) => {
                                session.send_disconnect(stream, "Unexpected packet");
                            }
                            _ => {}
                        }
                        break Err(e);
                    }
                }
            }
            Ok(None) => {
                // Need more data - read from stream
                let mut chunk = [0u8; 4096];
                match stream.read(&mut chunk) {
                    Ok(0) => break Err(SessionError::Disconnected),
                    Ok(n) => read_buffer.extend_from_slice(&chunk[..n]),
                    Err(e) => {
                        // Treat timeout as non-fatal: continue the loop to
                        // poll for tick events (KeepAlive, etc.)
                        if e.kind() == std::io::ErrorKind::WouldBlock
                            || e.kind() == std::io::ErrorKind::TimedOut
                            || e.kind() == std::io::ErrorKind::Interrupted
                        {
                            continue;
                        }
                        break Err(SessionError::Io(e));
                    }
                }
            }
            Err(e) => {
                session.send_disconnect(stream, "Protocol error");
                break Err(e);
            }
        }
    };

    // session.drop() sends PlayerLeft via the Drop guard
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustbound_protocol::framing::PROTOCOL_MAX_FRAME_LENGTH;
    use std::sync::mpsc::channel;

    #[test]
    fn entity_id_allocator_is_sequential() {
        let allocator = EntityIdAllocator::new(1);
        assert_eq!(allocator.allocate(), 1);
        assert_eq!(allocator.allocate(), 2);
        assert_eq!(allocator.allocate(), 3);
    }

    #[test]
    fn degrees_to_angle_byte_basic() {
        assert_eq!(degrees_to_angle_byte(0.0), 0);
        assert_eq!(degrees_to_angle_byte(90.0), 64);
        assert_eq!(degrees_to_angle_byte(180.0), 128);
        assert_eq!(degrees_to_angle_byte(270.0), 192);
        assert_eq!(degrees_to_angle_byte(360.0), 0);
    }

    #[test]
    fn degrees_to_angle_byte_negative() {
        assert_eq!(degrees_to_angle_byte(-90.0), 192); // 270 degrees
        assert_eq!(degrees_to_angle_byte(-180.0), 128); // 180 degrees
    }

    #[test]
    fn degrees_to_angle_byte_wrap() {
        assert_eq!(degrees_to_angle_byte(720.0), 0); // 2 full turns
        assert_eq!(degrees_to_angle_byte(450.0), 64); // 90 + 360 = 90 normalized
    }

    #[test]
    fn entity_id_allocator_clone_shares_counter() {
        let allocator = EntityIdAllocator::new(100);
        let clone = allocator.clone();
        assert_eq!(allocator.allocate(), 100);
        assert_eq!(clone.allocate(), 101);
        assert_eq!(allocator.allocate(), 102);
    }

    #[test]
    fn player_session_sends_player_joined_to_tick() -> Result<(), Box<dyn std::error::Error>> {
        let (tx, rx) = channel::<TickMessage>();
        let config = SessionConfig {
            uuid: Uuid::new(0, 0),
            username: "TestPlayer".to_string(),
            gamemode: 0,
            max_frame_length: PROTOCOL_MAX_FRAME_LENGTH,
            read_timeout: Duration::from_secs(5),
            compression_threshold: -1,
            view_distance: 10,
            simulation_distance: 10,
            max_players: 20,
            keep_alive_timeout: Duration::from_secs(30),
            enabled_dimensions: crate::hakoniwa::DimensionSet::default(),
        };
        let session = PlayerSession::new(&config, 42, tx)?;

        let msg = rx.recv()?;
        match msg {
            TickMessage::PlayerJoined {
                entity_id,
                username,
                ..
            } => {
                assert_eq!(entity_id, 42);
                assert_eq!(username, "TestPlayer");
            }
            other => panic!("expected PlayerJoined, got {other:?}"),
        }

        // Drop should send PlayerLeft
        drop(session);
        let msg = rx.recv()?;
        match msg {
            TickMessage::PlayerLeft { entity_id } => assert_eq!(entity_id, 42),
            other => panic!("expected PlayerLeft, got {other:?}"),
        }
        Ok(())
    }

    #[test]
    fn player_session_receives_keepalive_event() -> Result<(), Box<dyn std::error::Error>> {
        let (tx, rx) = channel::<TickMessage>();
        let config = SessionConfig {
            uuid: Uuid::new(0, 0),
            username: "TestPlayer".to_string(),
            gamemode: 0,
            max_frame_length: PROTOCOL_MAX_FRAME_LENGTH,
            read_timeout: Duration::from_secs(5),
            compression_threshold: -1,
            view_distance: 10,
            simulation_distance: 10,
            max_players: 20,
            keep_alive_timeout: Duration::from_secs(30),
            enabled_dimensions: crate::hakoniwa::DimensionSet::default(),
        };
        let session = PlayerSession::new(&config, 1, tx)?;

        // Consume the PlayerJoined message to get the event sender
        let event_sender = match rx.recv()? {
            TickMessage::PlayerJoined { event_sender, .. } => event_sender,
            other => panic!("expected PlayerJoined, got {other:?}"),
        };

        // Send a KeepAlive event
        event_sender.send(SessionEvent::KeepAlive { payload: 42 })?;

        // Check that the session received it
        let event = session.event_receiver.try_recv()?;
        match event {
            SessionEvent::KeepAlive { payload } => assert_eq!(payload, 42),
            _ => panic!("expected KeepAlive event"),
        }

        Ok(())
    }

    #[test]
    fn session_error_display() {
        let error = SessionError::Disconnected;
        assert!(format!("{error}").contains("disconnected"));

        let error = SessionError::TeleportTimeout;
        assert!(format!("{error}").contains("timeout"));
    }

    #[test]
    fn minimal_registry_codec_is_valid_nbt() {
        let codec = minimal_registry_codec();
        // Root tag should be TAG_COMPOUND (0x0a)
        assert_eq!(codec[0], 0x0a);
        // Should contain dimension_type, biome, and chat_type registries
        let codec_str = String::from_utf8_lossy(&codec);
        assert!(codec_str.contains("minecraft:dimension_type"));
        assert!(codec_str.contains("minecraft:worldgen/biome"));
        assert!(codec_str.contains("minecraft:chat_type"));
    }

    #[test]
    fn try_decode_player_digging_packet() -> Result<(), Box<dyn std::error::Error>> {
        use rustbound_protocol::play::{PlayerDigging, PlayerDiggingAction, encode_player_digging};

        let dig = PlayerDigging {
            action: PlayerDiggingAction::StartDestroy,
            position: (10, 64, -5),
            face: 1,
            sequence: 0,
        };
        let mut wire = Vec::new();
        encode_player_digging(&dig, PROTOCOL_MAX_FRAME_LENGTH, &mut wire)?;

        let mut buffer = wire.clone();
        let packet = try_decode_play_packet(&mut buffer, PROTOCOL_MAX_FRAME_LENGTH, -1)?;
        assert!(packet.is_some(), "should decode PlayerDigging");
        assert!(buffer.is_empty(), "should consume all bytes");

        match packet {
            Some(PlayPacket::PlayerDigging(d)) => {
                assert_eq!(d.position, (10, 64, -5));
                assert_eq!(d.face, 1);
            }
            other => panic!("expected PlayerDigging, got {other:?}"),
        }
        Ok(())
    }

    #[test]
    fn try_decode_use_item_on_packet() -> Result<(), Box<dyn std::error::Error>> {
        use rustbound_protocol::play::{UseItemOn, encode_use_item_on};

        let place = UseItemOn {
            position: (0, 64, 0),
            face: 1, // top
            hand: 0,
            cursor_x: 0.5,
            cursor_y: 1.0,
            cursor_z: 0.5,
            inside_block: false,
            sequence: 0,
        };
        let mut wire = Vec::new();
        encode_use_item_on(&place, PROTOCOL_MAX_FRAME_LENGTH, &mut wire)?;

        let mut buffer = wire.clone();
        let packet = try_decode_play_packet(&mut buffer, PROTOCOL_MAX_FRAME_LENGTH, -1)?;
        assert!(packet.is_some(), "should decode UseItemOn");
        assert!(buffer.is_empty(), "should consume all bytes");

        match packet {
            Some(PlayPacket::UseItemOn(p)) => {
                assert_eq!(p.position, (0, 64, 0));
                assert_eq!(p.face, 1);
            }
            other => panic!("expected UseItemOn, got {other:?}"),
        }
        Ok(())
    }

    #[test]
    fn try_decode_unknown_packet_is_skipped() -> Result<(), Box<dyn std::error::Error>> {
        // Encode a frame with an unknown packet ID (0xFF)
        let mut wire = Vec::new();
        rustbound_protocol::framing::encode_frame(
            0xFF,
            &[0x01, 0x02, 0x03],
            PROTOCOL_MAX_FRAME_LENGTH,
            &mut wire,
        )?;

        let mut buffer = wire.clone();
        // Should not return an error - just None (skipped)
        let result = try_decode_play_packet(&mut buffer, PROTOCOL_MAX_FRAME_LENGTH, -1);
        assert!(result.is_ok(), "unknown packet should not error");
        // The packet should have been consumed (buffer empty)
        assert!(buffer.is_empty(), "unknown packet should be consumed");
        Ok(())
    }

    #[test]
    fn player_digging_start_destroy_sends_set_block() -> Result<(), Box<dyn std::error::Error>> {
        use rustbound_protocol::play::{PlayerDigging, PlayerDiggingAction};

        let (tx, rx) = channel::<TickMessage>();
        let config = SessionConfig {
            uuid: Uuid::new(0, 0),
            username: "TestPlayer".to_string(),
            gamemode: 1, // Creative
            max_frame_length: PROTOCOL_MAX_FRAME_LENGTH,
            read_timeout: Duration::from_secs(5),
            compression_threshold: -1,
            view_distance: 10,
            simulation_distance: 10,
            max_players: 20,
            keep_alive_timeout: Duration::from_secs(30),
            enabled_dimensions: crate::hakoniwa::DimensionSet::default(),
        };
        let mut session = PlayerSession::new(&config, 1, tx)?;

        // Consume PlayerJoined
        let _ = rx.recv()?;

        // Send a StartDestroy dig packet
        let ack = session.handle_play_packet(PlayPacket::PlayerDigging(PlayerDigging {
            action: PlayerDiggingAction::StartDestroy,
            position: (5, 10, 15),
            face: 1,
            sequence: 7,
        }))?;

        // Should return ACK sequence
        assert_eq!(ack, Some(7));

        // Should receive SetBlock with block_state=0 (air) in the digger's dimension.
        match rx.recv()? {
            TickMessage::SetBlock {
                entity_id: Some(1),
                position,
                block_state,
            } => {
                assert_eq!(position, (5, 10, 15));
                assert_eq!(block_state, 0);
            }
            other => panic!("expected SetBlock, got {other:?}"),
        }
        Ok(())
    }

    #[test]
    fn use_item_on_sends_set_block_on_adjacent_face() -> Result<(), Box<dyn std::error::Error>> {
        use rustbound_protocol::play::UseItemOn;

        let (tx, rx) = channel::<TickMessage>();
        let config = SessionConfig {
            uuid: Uuid::new(0, 0),
            username: "TestPlayer".to_string(),
            gamemode: 1, // Creative
            max_frame_length: PROTOCOL_MAX_FRAME_LENGTH,
            read_timeout: Duration::from_secs(5),
            compression_threshold: -1,
            view_distance: 10,
            simulation_distance: 10,
            max_players: 20,
            keep_alive_timeout: Duration::from_secs(30),
            enabled_dimensions: crate::hakoniwa::DimensionSet::default(),
        };
        let mut session = PlayerSession::new(&config, 1, tx)?;

        // Consume PlayerJoined
        let _ = rx.recv()?;

        // Place on top face (face=1) of block at (0,64,0)
        let ack = session.handle_play_packet(PlayPacket::UseItemOn(UseItemOn {
            position: (0, 64, 0),
            face: 1,
            hand: 0,
            cursor_x: 0.5,
            cursor_y: 1.0,
            cursor_z: 0.5,
            inside_block: false,
            sequence: 3,
        }))?;

        // Should return ACK sequence
        assert_eq!(ack, Some(3));

        // Should receive PlaceBlock (tick loop looks up held item)
        match rx.recv()? {
            TickMessage::PlaceBlock {
                entity_id,
                position,
                face,
            } => {
                assert_eq!(entity_id, 1);
                assert_eq!(position, (0, 64, 0));
                assert_eq!(face, 1);
            }
            other => panic!("expected PlaceBlock, got {other:?}"),
        }
        Ok(())
    }

    #[test]
    fn survival_dig_does_not_modify_world_but_acks() -> Result<(), Box<dyn std::error::Error>> {
        use rustbound_protocol::play::{PlayerDigging, PlayerDiggingAction};

        let (tx, rx) = channel::<TickMessage>();
        let config = SessionConfig {
            uuid: Uuid::new(0, 0),
            username: "TestPlayer".to_string(),
            gamemode: 0, // Survival
            max_frame_length: PROTOCOL_MAX_FRAME_LENGTH,
            read_timeout: Duration::from_secs(5),
            compression_threshold: -1,
            view_distance: 10,
            simulation_distance: 10,
            max_players: 20,
            keep_alive_timeout: Duration::from_secs(30),
            enabled_dimensions: crate::hakoniwa::DimensionSet::default(),
        };
        let mut session = PlayerSession::new(&config, 1, tx)?;

        // Consume PlayerJoined
        let _ = rx.recv()?;

        // Send a StartDestroy dig packet in Survival mode
        let ack = session.handle_play_packet(PlayPacket::PlayerDigging(PlayerDigging {
            action: PlayerDiggingAction::StartDestroy,
            position: (5, 10, 15),
            face: 1,
            sequence: 99,
        }))?;

        // Should still return ACK sequence (protocol requires it)
        assert_eq!(ack, Some(99));

        // Should receive DigBlock (forwarded to tick loop for break progress)
        match rx.try_recv() {
            Ok(TickMessage::DigBlock {
                action, position, ..
            }) => {
                assert_eq!(action, 0, "should be StartDestroy action");
                assert_eq!(position, (5, 10, 15));
            }
            Ok(msg) => panic!("expected DigBlock, got {msg:?}"),
            Err(e) => panic!("channel error: {e}"),
        }
        Ok(())
    }

    #[test]
    fn survival_use_item_on_forwards_place_block_for_chests() -> Result<(), Box<dyn std::error::Error>>
    {
        use rustbound_protocol::play::UseItemOn;

        let (tx, rx) = channel::<TickMessage>();
        let config = SessionConfig {
            uuid: Uuid::new(0, 0),
            username: "TestPlayer".to_string(),
            gamemode: 0, // Survival
            max_frame_length: PROTOCOL_MAX_FRAME_LENGTH,
            read_timeout: Duration::from_secs(5),
            compression_threshold: -1,
            view_distance: 10,
            simulation_distance: 10,
            max_players: 20,
            keep_alive_timeout: Duration::from_secs(30),
            enabled_dimensions: crate::hakoniwa::DimensionSet::default(),
        };
        let mut session = PlayerSession::new(&config, 1, tx)?;

        // Consume PlayerJoined
        let _ = rx.recv()?;

        // Send a UseItemOn packet in Survival mode (needed to open chests)
        let ack = session.handle_play_packet(PlayPacket::UseItemOn(UseItemOn {
            position: (0, 64, 0),
            face: 1,
            hand: 0,
            cursor_x: 0.5,
            cursor_y: 1.0,
            cursor_z: 0.5,
            inside_block: false,
            sequence: 55,
        }))?;

        // Should still return ACK sequence
        assert_eq!(ack, Some(55));

        // Tick loop decides open-vs-place; session always forwards PlaceBlock.
        match rx.try_recv() {
            Ok(TickMessage::PlaceBlock {
                entity_id,
                position,
                face,
            }) => {
                assert_eq!(entity_id, 1);
                assert_eq!(position, (0, 64, 0));
                assert_eq!(face, 1);
            }
            Ok(msg) => panic!("expected PlaceBlock, got {msg:?}"),
            Err(e) => panic!("channel error: {e}"),
        }
        Ok(())
    }

    #[test]
    fn keep_alive_timeout_not_triggered_initially() -> Result<(), Box<dyn std::error::Error>> {
        let (tx, _rx) = channel::<TickMessage>();
        let config = SessionConfig {
            uuid: Uuid::new(0, 0),
            username: "TestPlayer".to_string(),
            gamemode: 0,
            max_frame_length: PROTOCOL_MAX_FRAME_LENGTH,
            read_timeout: Duration::from_secs(5),
            compression_threshold: -1,
            view_distance: 10,
            simulation_distance: 10,
            max_players: 20,
            keep_alive_timeout: Duration::from_secs(30),
            enabled_dimensions: crate::hakoniwa::DimensionSet::default(),
        };
        let session = PlayerSession::new(&config, 1, tx)?;
        assert!(!session.is_keep_alive_timed_out());
        Ok(())
    }

    #[test]
    fn keep_alive_timeout_triggers_after_expiry() -> Result<(), Box<dyn std::error::Error>> {
        let (tx, _rx) = channel::<TickMessage>();
        let config = SessionConfig {
            uuid: Uuid::new(0, 0),
            username: "TestPlayer".to_string(),
            gamemode: 0,
            max_frame_length: PROTOCOL_MAX_FRAME_LENGTH,
            read_timeout: Duration::from_secs(5),
            compression_threshold: -1,
            view_distance: 10,
            simulation_distance: 10,
            max_players: 20,
            keep_alive_timeout: Duration::from_millis(1),
            enabled_dimensions: crate::hakoniwa::DimensionSet::default(),
        };
        let mut session = PlayerSession::new(&config, 1, tx)?;

        use std::net::{TcpListener, TcpStream};
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let addr = listener.local_addr()?;
        let client = TcpStream::connect(addr)?;
        client.set_nonblocking(true).ok();
        let mut server = listener.accept()?.0;
        server.set_nonblocking(true).ok();

        session.send_keep_alive(&mut server, 42)?;
        std::thread::sleep(Duration::from_millis(10));
        assert!(session.is_keep_alive_timed_out());
        Ok(())
    }

    #[test]
    fn keep_alive_response_clears_timeout() -> Result<(), Box<dyn std::error::Error>> {
        let (tx, _rx) = channel::<TickMessage>();
        let config = SessionConfig {
            uuid: Uuid::new(0, 0),
            username: "TestPlayer".to_string(),
            gamemode: 0,
            max_frame_length: PROTOCOL_MAX_FRAME_LENGTH,
            read_timeout: Duration::from_secs(5),
            compression_threshold: -1,
            view_distance: 10,
            simulation_distance: 10,
            max_players: 20,
            keep_alive_timeout: Duration::from_secs(30),
            enabled_dimensions: crate::hakoniwa::DimensionSet::default(),
        };
        let mut session = PlayerSession::new(&config, 1, tx)?;

        use std::net::{TcpListener, TcpStream};
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let addr = listener.local_addr()?;
        let client = TcpStream::connect(addr)?;
        client.set_nonblocking(true).ok();
        let mut server = listener.accept()?.0;
        server.set_nonblocking(true).ok();

        session.send_keep_alive(&mut server, 99)?;
        assert!(!session.is_keep_alive_timed_out());
        session.handle_play_packet(PlayPacket::KeepAliveServerbound(KeepAlive { payload: 99 }))?;
        assert!(!session.is_keep_alive_timed_out());
        Ok(())
    }

    #[test]
    fn keep_alive_wrong_payload_does_not_clear_timeout() -> Result<(), Box<dyn std::error::Error>> {
        let (tx, _rx) = channel::<TickMessage>();
        let config = SessionConfig {
            uuid: Uuid::new(0, 0),
            username: "TestPlayer".to_string(),
            gamemode: 0,
            max_frame_length: PROTOCOL_MAX_FRAME_LENGTH,
            read_timeout: Duration::from_secs(5),
            compression_threshold: -1,
            view_distance: 10,
            simulation_distance: 10,
            max_players: 20,
            keep_alive_timeout: Duration::from_millis(1),
            enabled_dimensions: crate::hakoniwa::DimensionSet::default(),
        };
        let mut session = PlayerSession::new(&config, 1, tx)?;

        use std::net::{TcpListener, TcpStream};
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let addr = listener.local_addr()?;
        let client = TcpStream::connect(addr)?;
        client.set_nonblocking(true).ok();
        let mut server = listener.accept()?.0;
        server.set_nonblocking(true).ok();

        session.send_keep_alive(&mut server, 100)?;
        session.handle_play_packet(PlayPacket::KeepAliveServerbound(KeepAlive { payload: 999 }))?;
        std::thread::sleep(Duration::from_millis(10));
        assert!(session.is_keep_alive_timed_out());
        Ok(())
    }
}
