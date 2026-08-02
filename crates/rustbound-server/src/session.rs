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
    ChangeDifficulty, ClientInformation, ConfirmTeleportation, DisconnectPlay, EntityEvent,
    GameEvent, GameMode, JoinGame, KeepAlive, PlayDecodeOutcome, PlayError, PlayPacket,
    PlayerAbilities, PluginMessageClientbound, SetCenterChunk, SetDefaultSpawnPosition,
    SetHeldItem, SetRenderDistance, SetSimulationDistance, SynchronizePlayerPosition,
    decode_client_information, decode_confirm_teleportation, decode_keep_alive_serverbound,
    decode_player_digging, decode_set_player_position, decode_set_player_position_and_rotation,
    decode_set_player_rotation, decode_use_item_on, encode_change_difficulty, encode_chunk_data,
    encode_disconnect_play, encode_entity_event, encode_game_event, encode_join_game,
    encode_keep_alive_clientbound, encode_player_abilities, encode_plugin_message_clientbound,
    encode_set_center_chunk, encode_set_default_spawn_position, encode_set_held_item,
    encode_set_render_distance, encode_set_simulation_distance, encode_synchronize_player_position,
};
use rustbound_protocol::primitives::Uuid;

use crate::tick::TickMessage;

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
        let join_game = JoinGame {
            entity_id: self.entity_id,
            is_hardcore: false,
            gamemode: self.gamemode,
            previous_gamemode: None,
            dimension_names: vec!["minecraft:overworld".to_string()],
            registry_codec: minimal_registry_codec(),
            dimension_type: "minecraft:overworld".to_string(),
            dimension_name: "minecraft:overworld".to_string(),
            hashed_seed: 0,
            max_players: 20,
            view_distance: 10,
            simulation_distance: 10,
            reduce_debug_info: false,
            enable_respawn_screen: true,
            is_debug: false,
            is_flat: true,
        };
        let mut wire = Vec::new();
        encode_join_game(&join_game, self.max_frame_length, &mut wire)?;
        self.send_wire(stream, &wire)?;
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
    /// 3. Player Abilities
    /// 4. Set Held Item
    /// 5. Entity Event (player status)
    /// 6. Set Default Spawn Position
    /// 7. Set Center Chunk
    /// 8. Set Render Distance
    /// 9. Set Simulation Distance
    /// 10. Game Event (start waiting for level chunks)
    pub fn send_join_sequence(&self, stream: &mut TcpStream) -> Result<(), SessionError> {
        let mfl = self.max_frame_length;

        // 1. Plugin Message (brand)  E0x17
        let brand = PluginMessageClientbound {
            channel: "minecraft:brand".to_string(),
            data: b"rustbound".to_vec(),
        };
        let mut wire = Vec::new();
        encode_plugin_message_clientbound(&brand, mfl, &mut wire)?;
        self.send_wire(stream, &wire)?;

        // 2. Change Difficulty  E0x0C
        let diff = ChangeDifficulty {
            difficulty: 1, // Easy
            locked: false,
        };
        wire.clear();
        encode_change_difficulty(&diff, mfl, &mut wire)?;
        self.send_wire(stream, &wire)?;

        // 3. Player Abilities  E0x34
        let abilities = PlayerAbilities {
            flags: 0x04, // allow flying bit
            flying_speed: 0.05,
            fov_modifier: 0.1,
        };
        wire.clear();
        encode_player_abilities(&abilities, mfl, &mut wire)?;
        self.send_wire(stream, &wire)?;

        // 4. Set Held Item  E0x4D
        let held = SetHeldItem { slot: 0 };
        wire.clear();
        encode_set_held_item(&held, mfl, &mut wire)?;
        self.send_wire(stream, &wire)?;

        // 5. Entity Event (player status: 28 = op permission level 4)  E0x1C
        let entity_event = EntityEvent {
            entity_id: self.entity_id,
            entity_status: 28,
        };
        wire.clear();
        encode_entity_event(&entity_event, mfl, &mut wire)?;
        self.send_wire(stream, &wire)?;

        // 6. Set Default Spawn Position  E0x50
        let spawn = SetDefaultSpawnPosition {
            location: (0, 64, 0),
            angle: 0.0,
        };
        wire.clear();
        encode_set_default_spawn_position(&spawn, mfl, &mut wire)?;
        self.send_wire(stream, &wire)?;

        // 7. Set Center Chunk  E0x4E
        let center = SetCenterChunk {
            chunk_x: 0,
            chunk_z: 0,
        };
        wire.clear();
        encode_set_center_chunk(&center, mfl, &mut wire)?;
        self.send_wire(stream, &wire)?;

        // 8. Set Render Distance  E0x4F
        let render = SetRenderDistance { view_distance: 10 };
        wire.clear();
        encode_set_render_distance(&render, mfl, &mut wire)?;
        self.send_wire(stream, &wire)?;

        // 9. Set Simulation Distance  E0x5C
        let sim = SetSimulationDistance {
            simulation_distance: 10,
        };
        wire.clear();
        encode_set_simulation_distance(&sim, mfl, &mut wire)?;
        self.send_wire(stream, &wire)?;

        // 10. Game Event (13 = Start waiting for level chunks)  E0x1F
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
            let chunk_data = crate::chunk::build_chunk_data_packet(pos.x, pos.z);
            let mut wire = Vec::new();
            encode_chunk_data(&chunk_data, mfl, &mut wire)?;
            self.send_wire(stream, &wire)?;
        }

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

    /// Sends a KeepAlive packet to the client.
    pub fn send_keep_alive(
        &self,
        stream: &mut TcpStream,
        payload: i64,
    ) -> Result<(), SessionError> {
        let packet = KeepAlive { payload };
        let mut wire = Vec::new();
        encode_keep_alive_clientbound(&packet, self.max_frame_length, &mut wire)?;
        self.send_wire(stream, &wire)?;
        Ok(())
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
    pub fn poll_events(&self, stream: &mut TcpStream) -> Result<bool, SessionError> {
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
            PlayPacket::KeepAliveServerbound(KeepAlive { payload: _ }) => {
                // KeepAlive response received - no action needed for now
                Ok(None)
            }
            PlayPacket::ClientInformation(ClientInformation { view_distance, .. }) => {
                // Store client view distance - for now just accept it
                // Phase C will use min(server, client) view distance for chunk loading
                let _ = view_distance;
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
                // Survival: deny (no break progress yet) but still ACK
                if self.gamemode == GameMode::Creative
                    && matches!(
                        dig.action,
                        rustbound_protocol::play::PlayerDiggingAction::StartDestroy
                    )
                {
                    // Set block to air (state 0)
                    let _ = self.tick_sender.send(TickMessage::SetBlock {
                        position: dig.position,
                        block_state: 0,
                    });
                }
                // Always ACK the dig packet (protocol requires it)
                Ok(Some(dig.sequence))
            }
            PlayPacket::UseItemOn(place) => {
                // Creative mode: place block (stone = state 1) on the face
                // Survival: deny (no placement yet) but still ACK
                if self.gamemode == GameMode::Creative {
                    let (x, y, z) = place.position;
                    let target = match place.face {
                        0 => (x, y - 1, z), // bottom
                        1 => (x, y + 1, z), // top
                        2 => (x, y, z - 1), // north
                        3 => (x, y, z + 1), // south
                        4 => (x - 1, y, z), // west
                        5 => (x + 1, y, z), // east
                        _ => (x, y, z),
                    };
                    let _ = self.tick_sender.send(TickMessage::SetBlock {
                        position: target,
                        block_state: 1, // stone
                    });
                }
                // Always ACK the place packet (protocol requires it)
                Ok(Some(place.sequence))
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
    session.send_join_game(stream)?;

    // Send vanilla join-sequence packets (brand, abilities, spawn pos, etc.)
    session.send_join_sequence(stream)?;

    // Send initial chunk columns (radius 2 for bootstrapping)
    session.send_initial_chunks(stream, 2)?;

    // Send Synchronize Player Position
    session.send_synchronize_position(stream)?;

    // Main play loop
    let mut read_buffer = Vec::with_capacity(4096);
    let result = loop {
        // Poll for tick events (KeepAlive, etc.)
        if let Err(e) = session.poll_events(stream) {
            break Err(e);
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

        // Should receive SetBlock with block_state=0 (air)
        match rx.recv()? {
            TickMessage::SetBlock {
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

        // Should receive SetBlock at (0,65,0) with block_state=1 (stone)
        match rx.recv()? {
            TickMessage::SetBlock {
                position,
                block_state,
            } => {
                assert_eq!(position, (0, 65, 0));
                assert_eq!(block_state, 1);
            }
            other => panic!("expected SetBlock, got {other:?}"),
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

        // Should NOT receive SetBlock (Survival doesn't instant-break)
        match rx.try_recv() {
            Err(std::sync::mpsc::TryRecvError::Empty) => {} // Good: no SetBlock
            Ok(msg) => panic!("Survival dig should not send SetBlock, got {msg:?}"),
            Err(e) => panic!("channel error: {e}"),
        }
        Ok(())
    }

    #[test]
    fn survival_place_does_not_modify_world_but_acks() -> Result<(), Box<dyn std::error::Error>> {
        use rustbound_protocol::play::UseItemOn;

        let (tx, rx) = channel::<TickMessage>();
        let config = SessionConfig {
            uuid: Uuid::new(0, 0),
            username: "TestPlayer".to_string(),
            gamemode: 0, // Survival
            max_frame_length: PROTOCOL_MAX_FRAME_LENGTH,
            read_timeout: Duration::from_secs(5),
            compression_threshold: -1,
        };
        let mut session = PlayerSession::new(&config, 1, tx)?;

        // Consume PlayerJoined
        let _ = rx.recv()?;

        // Send a UseItemOn packet in Survival mode
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

        // Should NOT receive SetBlock
        match rx.try_recv() {
            Err(std::sync::mpsc::TryRecvError::Empty) => {} // Good: no SetBlock
            Ok(msg) => panic!("Survival place should not send SetBlock, got {msg:?}"),
            Err(e) => panic!("channel error: {e}"),
        }
        Ok(())
    }
}
