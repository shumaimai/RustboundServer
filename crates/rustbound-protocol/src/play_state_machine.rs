//! Play state machine orchestration for protocol 763.
//!
//! This module models the play flow as a state machine that ties together the
//! individual play packet codecs. It does not perform network I/O; instead, it
//! operates on in-memory buffers and produces the packets that the server
//! should send, along with state transitions.

use std::fmt;

use crate::play::{
    DisconnectPlay, GameMode, JoinGame, PlayError, encode_disconnect_play, encode_join_game,
    encode_keep_alive_clientbound, encode_synchronize_player_position,
};
use crate::primitives::Uuid;
use crate::state::ProtocolState;

/// The internal sub-state of the Play state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PlayPhase {
    /// Waiting for the client to confirm the teleport after Join Game.
    AwaitingTeleportConfirm,
    /// Active play: processing keep alive and position updates.
    Active,
    /// The connection has been disconnected during play.
    Disconnected,
}

/// The result of processing a play event.
#[derive(Debug)]
pub enum PlayStepResult {
    /// The play flow should continue; the server should send these packets.
    Continue {
        /// The packets the server should send (already encoded).
        outgoing: Vec<Vec<u8>>,
        /// The new play phase.
        phase: PlayPhase,
    },
    /// The connection should be disconnected.
    Disconnect {
        /// The encoded Disconnect packet to send.
        outgoing: Vec<u8>,
        /// The new protocol state (Closed).
        new_state: ProtocolState,
    },
}

/// An error produced by the play state machine.
#[derive(Debug)]
pub enum PlayStateMachineError {
    /// A play packet codec error.
    Play(PlayError),
    /// The state machine has already reached a terminal state.
    Terminal,
    /// An unexpected event occurred in the current phase.
    UnexpectedEvent {
        /// The current play phase.
        phase: PlayPhase,
        /// What was expected.
        expected: &'static str,
    },
    /// A custom protocol-level error message.
    Custom(String),
}

impl fmt::Display for PlayStateMachineError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Play(error) => write!(formatter, "play error: {error}"),
            Self::Terminal => {
                formatter.write_str("play state machine has reached a terminal state")
            }
            Self::UnexpectedEvent { phase, expected } => {
                write!(
                    formatter,
                    "unexpected event in phase {phase:?}, expected {expected}"
                )
            }
            Self::Custom(message) => write!(formatter, "{message}"),
        }
    }
}

impl std::error::Error for PlayStateMachineError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Play(error) => Some(error),
            _ => None,
        }
    }
}

impl From<PlayError> for PlayStateMachineError {
    fn from(error: PlayError) -> Self {
        Self::Play(error)
    }
}

/// Configuration for the play state machine.
#[derive(Debug, Clone)]
pub struct PlayConfig {
    /// The maximum frame length for encoding.
    pub max_frame_length: usize,
    /// The player's entity ID.
    pub entity_id: i32,
    /// The player's gamemode.
    pub gamemode: GameMode,
    /// The player's username.
    pub username: String,
    /// The player's UUID.
    pub uuid: Uuid,
    /// The dimension name.
    pub dimension_name: String,
    /// The hashed seed.
    pub hashed_seed: i64,
    /// The max players.
    pub max_players: i32,
    /// The view distance.
    pub view_distance: i32,
    /// The simulation distance.
    pub simulation_distance: i32,
}

/// The play state machine.
#[derive(Debug, Clone)]
pub struct PlayStateMachine {
    phase: PlayPhase,
    config: PlayConfig,
    /// The last keep-alive payload sent.
    last_keep_alive_payload: Option<i64>,
    /// The player's current position.
    player_x: f64,
    player_y: f64,
    player_z: f64,
    /// The player's current rotation.
    player_yaw: f32,
    player_pitch: f32,
}

impl PlayStateMachine {
    /// Creates a new play state machine and produces the initial Join Game
    /// and Synchronize Player Position packets.
    pub fn new(config: PlayConfig) -> Result<(Self, Vec<Vec<u8>>), PlayStateMachineError> {
        let mut outgoing = Vec::new();

        // Send Join Game
        let join_game = JoinGame {
            entity_id: config.entity_id,
            is_hardcore: false,
            gamemode: config.gamemode,
            previous_gamemode: None,
            dimension_names: vec![config.dimension_name.clone()],
            registry_codec: crate::registry_codec::build_registry_codec(),
            dimension_type: config.dimension_name.clone(),
            dimension_name: config.dimension_name.clone(),
            hashed_seed: config.hashed_seed,
            max_players: config.max_players,
            view_distance: config.view_distance,
            simulation_distance: config.simulation_distance,
            reduce_debug_info: false,
            enable_respawn_screen: true,
            is_debug: false,
            is_flat: false,
            has_death_location: false,
            death_dimension_name: String::new(),
            death_location: (0, 0, 0),
            portal_cooldown: 0,
        };
        let mut join_wire = Vec::new();
        encode_join_game(&join_game, config.max_frame_length, &mut join_wire)?;
        outgoing.push(join_wire);

        // Send Synchronize Player Position (spawn at 0, 64, 0)
        let sync = crate::play::SynchronizePlayerPosition {
            x: 0.0,
            y: 64.0,
            z: 0.0,
            yaw: 0.0,
            pitch: 0.0,
            flags: 0x00,
            teleport_id: 0,
        };
        let mut sync_wire = Vec::new();
        encode_synchronize_player_position(&sync, config.max_frame_length, &mut sync_wire)?;
        outgoing.push(sync_wire);

        let sm = Self {
            phase: PlayPhase::AwaitingTeleportConfirm,
            config,
            last_keep_alive_payload: None,
            player_x: 0.0,
            player_y: 64.0,
            player_z: 0.0,
            player_yaw: 0.0,
            player_pitch: 0.0,
        };

        Ok((sm, outgoing))
    }

    /// Returns the current play phase.
    pub fn phase(&self) -> PlayPhase {
        self.phase
    }

    /// Sends a keep-alive to the client.
    pub fn send_keep_alive(&mut self, payload: i64) -> Result<Vec<u8>, PlayStateMachineError> {
        if self.phase == PlayPhase::Disconnected {
            return Err(PlayStateMachineError::Terminal);
        }
        self.last_keep_alive_payload = Some(payload);
        let mut wire = Vec::new();
        encode_keep_alive_clientbound(
            &crate::play::KeepAlive { payload },
            self.config.max_frame_length,
            &mut wire,
        )?;
        Ok(wire)
    }

    /// Handles a serverbound Keep Alive response.
    pub fn handle_keep_alive_response(
        &mut self,
        payload: i64,
    ) -> Result<(), PlayStateMachineError> {
        if self.phase == PlayPhase::Disconnected {
            return Err(PlayStateMachineError::Terminal);
        }
        match self.last_keep_alive_payload {
            Some(expected) if expected == payload => {
                self.last_keep_alive_payload = None;
                Ok(())
            }
            Some(expected) => Err(PlayStateMachineError::Custom(format!(
                "keep alive mismatch: expected {expected}, got {payload}"
            ))),
            None => Err(PlayStateMachineError::UnexpectedEvent {
                phase: self.phase,
                expected: "no pending keep alive",
            }),
        }
    }

    /// Handles a serverbound Set Player Position update.
    pub fn handle_player_position(
        &mut self,
        x: f64,
        y: f64,
        z: f64,
        _on_ground: bool,
    ) -> Result<(), PlayStateMachineError> {
        if self.phase == PlayPhase::Disconnected {
            return Err(PlayStateMachineError::Terminal);
        }
        self.player_x = x;
        self.player_y = y;
        self.player_z = z;
        Ok(())
    }

    /// Handles a serverbound Set Player Position and Rotation update.
    pub fn handle_player_position_and_rotation(
        &mut self,
        x: f64,
        y: f64,
        z: f64,
        yaw: f32,
        pitch: f32,
        _on_ground: bool,
    ) -> Result<(), PlayStateMachineError> {
        if self.phase == PlayPhase::Disconnected {
            return Err(PlayStateMachineError::Terminal);
        }
        self.player_x = x;
        self.player_y = y;
        self.player_z = z;
        self.player_yaw = yaw;
        self.player_pitch = pitch;
        Ok(())
    }

    /// Handles a serverbound Set Player Rotation update.
    pub fn handle_player_rotation(
        &mut self,
        yaw: f32,
        pitch: f32,
        _on_ground: bool,
    ) -> Result<(), PlayStateMachineError> {
        if self.phase == PlayPhase::Disconnected {
            return Err(PlayStateMachineError::Terminal);
        }
        self.player_yaw = yaw;
        self.player_pitch = pitch;
        Ok(())
    }

    /// Transitions to the Active phase (after teleport confirm).
    pub fn confirm_teleport(&mut self) -> Result<(), PlayStateMachineError> {
        if self.phase == PlayPhase::Disconnected {
            return Err(PlayStateMachineError::Terminal);
        }
        if self.phase != PlayPhase::AwaitingTeleportConfirm {
            return Err(PlayStateMachineError::UnexpectedEvent {
                phase: self.phase,
                expected: "teleport confirmation",
            });
        }
        self.phase = PlayPhase::Active;
        Ok(())
    }

    /// Disconnects the player with a reason.
    pub fn disconnect(&mut self, reason: &str) -> Result<PlayStepResult, PlayStateMachineError> {
        if self.phase == PlayPhase::Disconnected {
            return Err(PlayStateMachineError::Terminal);
        }
        let disconnect = DisconnectPlay {
            reason: reason.to_string(),
        };
        let mut wire = Vec::new();
        encode_disconnect_play(&disconnect, self.config.max_frame_length, &mut wire)?;
        self.phase = PlayPhase::Disconnected;
        Ok(PlayStepResult::Disconnect {
            outgoing: wire,
            new_state: ProtocolState::Closed,
        })
    }

    /// Returns the player's current position.
    pub fn player_position(&self) -> (f64, f64, f64) {
        (self.player_x, self.player_y, self.player_z)
    }

    /// Returns the player's current rotation.
    pub fn player_rotation(&self) -> (f32, f32) {
        (self.player_yaw, self.player_pitch)
    }
}

#[cfg(test)]
mod tests {
    use super::{PlayConfig, PlayPhase, PlayStateMachine, PlayStepResult};
    use crate::play::GameMode;
    use crate::primitives::Uuid;
    use crate::state::ProtocolState;

    const TEST_MAX_FRAME: usize = 1048576;

    fn test_config() -> PlayConfig {
        PlayConfig {
            max_frame_length: TEST_MAX_FRAME,
            entity_id: 42,
            gamemode: GameMode::Survival,
            username: "Steve".to_string(),
            uuid: Uuid::new(0, 0),
            dimension_name: "minecraft:overworld".to_string(),
            hashed_seed: 0,
            max_players: 20,
            view_distance: 10,
            simulation_distance: 10,
        }
    }

    #[test]
    fn play_state_machine_initializes_with_join_game_and_sync()
    -> Result<(), Box<dyn std::error::Error>> {
        let (sm, outgoing) = PlayStateMachine::new(test_config())?;
        assert_eq!(sm.phase(), PlayPhase::AwaitingTeleportConfirm);
        assert_eq!(outgoing.len(), 2); // Join Game + Synchronize Player Position
        Ok(())
    }

    #[test]
    fn play_state_machine_teleport_confirm_transitions_to_active()
    -> Result<(), Box<dyn std::error::Error>> {
        let (mut sm, _) = PlayStateMachine::new(test_config())?;
        sm.confirm_teleport()?;
        assert_eq!(sm.phase(), PlayPhase::Active);
        Ok(())
    }

    #[test]
    fn play_state_machine_keep_alive_flow() -> Result<(), Box<dyn std::error::Error>> {
        let (mut sm, _) = PlayStateMachine::new(test_config())?;
        sm.confirm_teleport()?;

        let wire = sm.send_keep_alive(12345)?;
        assert!(!wire.is_empty());

        sm.handle_keep_alive_response(12345)?;
        assert!(sm.last_keep_alive_payload.is_none());
        Ok(())
    }

    #[test]
    fn play_state_machine_keep_alive_mismatch_is_rejected() -> Result<(), Box<dyn std::error::Error>>
    {
        let (mut sm, _) = PlayStateMachine::new(test_config())?;
        sm.confirm_teleport()?;
        sm.send_keep_alive(12345)?;
        let result = sm.handle_keep_alive_response(99999);
        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn play_state_machine_position_update() -> Result<(), Box<dyn std::error::Error>> {
        let (mut sm, _) = PlayStateMachine::new(test_config())?;
        sm.confirm_teleport()?;
        sm.handle_player_position(10.0, 70.0, -5.0, true)?;
        assert_eq!(sm.player_position(), (10.0, 70.0, -5.0));
        Ok(())
    }

    #[test]
    fn play_state_machine_position_and_rotation_update() -> Result<(), Box<dyn std::error::Error>> {
        let (mut sm, _) = PlayStateMachine::new(test_config())?;
        sm.confirm_teleport()?;
        sm.handle_player_position_and_rotation(100.0, 64.0, 200.0, 90.0, -45.0, false)?;
        assert_eq!(sm.player_position(), (100.0, 64.0, 200.0));
        assert_eq!(sm.player_rotation(), (90.0, -45.0));
        Ok(())
    }

    #[test]
    fn play_state_machine_rotation_update() -> Result<(), Box<dyn std::error::Error>> {
        let (mut sm, _) = PlayStateMachine::new(test_config())?;
        sm.confirm_teleport()?;
        sm.handle_player_rotation(180.0, 0.0, true)?;
        assert_eq!(sm.player_rotation(), (180.0, 0.0));
        Ok(())
    }

    #[test]
    fn play_state_machine_disconnect() -> Result<(), Box<dyn std::error::Error>> {
        let (mut sm, _) = PlayStateMachine::new(test_config())?;
        let result = sm.disconnect("Goodbye!")?;
        match result {
            PlayStepResult::Disconnect { new_state, .. } => {
                assert_eq!(new_state, ProtocolState::Closed);
            }
            _ => panic!("expected Disconnect"),
        }
        assert_eq!(sm.phase(), PlayPhase::Disconnected);
        Ok(())
    }

    #[test]
    fn play_state_machine_terminal_rejects_further_events() -> Result<(), Box<dyn std::error::Error>>
    {
        let (mut sm, _) = PlayStateMachine::new(test_config())?;
        sm.disconnect("bye")?;
        assert!(sm.send_keep_alive(0).is_err());
        assert!(sm.handle_player_position(0.0, 0.0, 0.0, true).is_err());
        Ok(())
    }

    #[test]
    fn play_state_machine_teleport_confirm_in_active_is_rejected()
    -> Result<(), Box<dyn std::error::Error>> {
        let (mut sm, _) = PlayStateMachine::new(test_config())?;
        sm.confirm_teleport()?;
        let result = sm.confirm_teleport();
        assert!(result.is_err());
        Ok(())
    }
}
