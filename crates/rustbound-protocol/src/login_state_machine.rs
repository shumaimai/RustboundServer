//! Login state machine orchestration for protocol 763.
//!
//! This module models the login flow as a state machine that ties together the
//! individual login packet codecs. It does not perform network I/O; instead, it
//! operates on in-memory buffers and produces the packets that the server
//! should send, along with state transitions.
//!
//! The login flow supports two paths:
//! - **Offline mode** (online-mode=false): Login Start -> Login Success -> Play
//! - **Online mode** (online-mode=true): Login Start -> Encryption Request ->
//!   Encryption Response -> Login Success -> Play
//!
//! Set Compression may be sent before Login Success in either path.

use std::fmt;

use crate::login::{
    EncryptionRequest, EncryptionResponse, LoginDisconnect, LoginError, LoginSuccess,
    decode_login_start, encode_encryption_request, encode_login_disconnect, encode_login_success,
};
use crate::state::ProtocolState;

/// The internal sub-state of the Login state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LoginPhase {
    /// Waiting for the client's Login Start packet.
    AwaitingLoginStart,
    /// Waiting for the client's Encryption Response (online mode only).
    AwaitingEncryptionResponse,
    /// Login has completed and the connection should transition to Play.
    LoginComplete,
    /// The connection has been disconnected during login.
    Disconnected,
}

/// The result of processing a serverbound login packet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginStepResult {
    /// The login flow should continue; the server should send these packets.
    Continue {
        /// The packets the server should send (already encoded).
        outgoing: Vec<Vec<u8>>,
        /// The new login phase.
        phase: LoginPhase,
    },
    /// Login succeeded; the connection should transition to Play.
    Success {
        /// The encoded Login Success packet to send.
        outgoing: Vec<u8>,
        /// The new protocol state (Play).
        new_state: ProtocolState,
    },
    /// The connection should be disconnected.
    Disconnect {
        /// The encoded Login Disconnect packet to send.
        outgoing: Vec<u8>,
        /// The new protocol state (Closed).
        new_state: ProtocolState,
    },
}

/// An error produced by the login state machine.
#[derive(Debug)]
pub enum LoginStateMachineError {
    /// A login packet codec error.
    Login(LoginError),
    /// A packet was received in a phase that does not accept it.
    UnexpectedPacket {
        /// The current login phase.
        phase: LoginPhase,
        /// What was received.
        expected: &'static str,
    },
    /// The state machine has already reached a terminal state.
    Terminal,
}

impl fmt::Display for LoginStateMachineError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Login(error) => write!(formatter, "login error: {error}"),
            Self::UnexpectedPacket { phase, expected } => {
                write!(
                    formatter,
                    "unexpected packet in phase {phase:?}, expected {expected}"
                )
            }
            Self::Terminal => {
                formatter.write_str("login state machine has reached a terminal state")
            }
        }
    }
}

impl std::error::Error for LoginStateMachineError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Login(error) => Some(error),
            _ => None,
        }
    }
}

impl From<LoginError> for LoginStateMachineError {
    fn from(error: LoginError) -> Self {
        Self::Login(error)
    }
}

/// Configuration for the login state machine.
#[derive(Debug, Clone)]
pub struct LoginConfig {
    /// Whether the server is in online mode (requires encryption).
    pub online_mode: bool,
    /// The compression threshold (-1 disables, >= 0 enables).
    pub compression_threshold: i32,
    /// The maximum frame length for encoding.
    pub max_frame_length: usize,
    /// The player UUID to use for offline-mode login.
    pub offline_uuid: crate::primitives::Uuid,
}

/// The login state machine.
#[derive(Debug, Clone)]
pub struct LoginStateMachine {
    phase: LoginPhase,
    config: LoginConfig,
    /// The username received in Login Start (if any).
    username: Option<String>,
}

impl LoginStateMachine {
    /// Creates a new login state machine in the `AwaitingLoginStart` phase.
    pub fn new(config: LoginConfig) -> Self {
        Self {
            phase: LoginPhase::AwaitingLoginStart,
            config,
            username: None,
        }
    }

    /// Returns the current login phase.
    pub fn phase(&self) -> LoginPhase {
        self.phase
    }

    /// Processes a serverbound Login Start packet.
    ///
    /// In offline mode, this produces Login Success and transitions to Play.
    /// In online mode, this produces an Encryption Request and waits for the
    /// Encryption Response.
    pub fn handle_login_start(
        &mut self,
        input: &mut &[u8],
    ) -> Result<LoginStepResult, LoginStateMachineError> {
        if self.phase == LoginPhase::Disconnected || self.phase == LoginPhase::LoginComplete {
            return Err(LoginStateMachineError::Terminal);
        }
        if self.phase != LoginPhase::AwaitingLoginStart {
            return Err(LoginStateMachineError::UnexpectedPacket {
                phase: self.phase,
                expected: "Login Start",
            });
        }

        let packet = match decode_login_start(input, self.config.max_frame_length)? {
            crate::login::LoginDecodeOutcome::Complete(crate::login::LoginPacket::LoginStart(
                start,
            )) => start,
            crate::login::LoginDecodeOutcome::Complete(_) => {
                return Err(LoginStateMachineError::UnexpectedPacket {
                    phase: self.phase,
                    expected: "Login Start",
                });
            }
            crate::login::LoginDecodeOutcome::Incomplete => {
                return Ok(LoginStepResult::Continue {
                    outgoing: Vec::new(),
                    phase: self.phase,
                });
            }
        };

        self.username = Some(packet.username.clone());

        if self.config.online_mode {
            // Online mode: send Encryption Request and wait for response.
            let request = EncryptionRequest {
                server_id: String::new(),
                public_key: vec![0x00; 128], // placeholder
                verify_token: vec![0x00; 4], // placeholder
            };
            let mut wire = Vec::new();
            encode_encryption_request(&request, self.config.max_frame_length, &mut wire)?;
            self.phase = LoginPhase::AwaitingEncryptionResponse;
            Ok(LoginStepResult::Continue {
                outgoing: vec![wire],
                phase: self.phase,
            })
        } else {
            // Offline mode: send Login Success and transition to Play.
            let success = LoginSuccess {
                uuid: self.config.offline_uuid,
                username: packet.username,
                properties: Vec::new(),
            };
            let mut wire = Vec::new();
            encode_login_success(&success, self.config.max_frame_length, &mut wire)?;
            self.phase = LoginPhase::LoginComplete;
            Ok(LoginStepResult::Success {
                outgoing: wire,
                new_state: ProtocolState::Play,
            })
        }
    }

    /// Processes a serverbound Encryption Response packet (online mode only).
    pub fn handle_encryption_response(
        &mut self,
        input: &mut &[u8],
    ) -> Result<LoginStepResult, LoginStateMachineError> {
        if self.phase == LoginPhase::Disconnected || self.phase == LoginPhase::LoginComplete {
            return Err(LoginStateMachineError::Terminal);
        }
        if self.phase != LoginPhase::AwaitingEncryptionResponse {
            return Err(LoginStateMachineError::UnexpectedPacket {
                phase: self.phase,
                expected: "Encryption Response",
            });
        }

        let _response: EncryptionResponse =
            match crate::login::decode_encryption_response(input, self.config.max_frame_length)? {
                crate::login::LoginDecodeOutcome::Complete(
                    crate::login::LoginPacket::EncryptionResponse(resp),
                ) => resp,
                crate::login::LoginDecodeOutcome::Complete(_) => {
                    return Err(LoginStateMachineError::UnexpectedPacket {
                        phase: self.phase,
                        expected: "Encryption Response",
                    });
                }
                crate::login::LoginDecodeOutcome::Incomplete => {
                    return Ok(LoginStepResult::Continue {
                        outgoing: Vec::new(),
                        phase: self.phase,
                    });
                }
            };

        // After encryption, send Login Success and transition to Play.
        let username = self.username.clone().unwrap_or_default();
        let success = LoginSuccess {
            uuid: self.config.offline_uuid,
            username,
            properties: Vec::new(),
        };
        let mut wire = Vec::new();
        encode_login_success(&success, self.config.max_frame_length, &mut wire)?;
        self.phase = LoginPhase::LoginComplete;
        Ok(LoginStepResult::Success {
            outgoing: wire,
            new_state: ProtocolState::Play,
        })
    }

    /// Disconnects the client with a reason.
    pub fn disconnect(&mut self, reason: &str) -> Result<LoginStepResult, LoginStateMachineError> {
        if self.phase == LoginPhase::Disconnected || self.phase == LoginPhase::LoginComplete {
            return Err(LoginStateMachineError::Terminal);
        }

        let disconnect = LoginDisconnect {
            reason: reason.to_string(),
        };
        let mut wire = Vec::new();
        encode_login_disconnect(&disconnect, self.config.max_frame_length, &mut wire)?;
        self.phase = LoginPhase::Disconnected;
        Ok(LoginStepResult::Disconnect {
            outgoing: wire,
            new_state: ProtocolState::Closed,
        })
    }

    /// Returns the username received in Login Start, if any.
    pub fn username(&self) -> Option<&str> {
        self.username.as_deref()
    }
}

/// Convenience function to decode a Login Start packet (re-exported).
pub fn decode_login_start_packet(
    input: &mut &[u8],
    max_frame_length: usize,
) -> Result<crate::login::LoginDecodeOutcome, LoginError> {
    decode_login_start(input, max_frame_length)
}

#[cfg(test)]
mod tests {
    use super::{LoginConfig, LoginPhase, LoginStateMachine, LoginStepResult};
    use crate::login::LoginError;
    use crate::primitives::Uuid;
    use crate::state::ProtocolState;

    const TEST_MAX_FRAME: usize = 65536;

    fn offline_uuid() -> Uuid {
        Uuid::from_be_bytes([
            0x06, 0x9a, 0x64, 0x8f, 0x86, 0x4c, 0x4e, 0x47, 0xa1, 0x0b, 0x6c, 0xd3, 0x8b, 0x6e,
            0x4c, 0x21,
        ])
    }

    fn encode_login_start_packet(username: &str, uuid: Uuid) -> Result<Vec<u8>, LoginError> {
        let mut wire = Vec::new();
        crate::login::encode_login_start(
            &crate::login::LoginStart {
                username: username.to_string(),
                uuid,
            },
            TEST_MAX_FRAME,
            &mut wire,
        )?;
        Ok(wire)
    }

    fn encode_encryption_response_packet() -> Result<Vec<u8>, LoginError> {
        let mut wire = Vec::new();
        crate::login::encode_encryption_response(
            &crate::login::EncryptionResponse {
                shared_secret: vec![0x42; 16],
                verify_token: vec![0xde, 0xad, 0xbe, 0xef],
            },
            TEST_MAX_FRAME,
            &mut wire,
        )?;
        Ok(wire)
    }

    #[test]
    fn offline_mode_login_flow() -> Result<(), Box<dyn std::error::Error>> {
        let config = LoginConfig {
            online_mode: false,
            compression_threshold: -1,
            max_frame_length: TEST_MAX_FRAME,
            offline_uuid: offline_uuid(),
        };
        let mut sm = LoginStateMachine::new(config);
        assert_eq!(sm.phase(), LoginPhase::AwaitingLoginStart);

        let wire = encode_login_start_packet("Steve", offline_uuid())?;
        let mut input = wire.as_slice();
        let result = sm.handle_login_start(&mut input)?;

        match result {
            LoginStepResult::Success { new_state, .. } => {
                assert_eq!(new_state, ProtocolState::Play);
            }
            _ => panic!("expected Success"),
        }
        assert_eq!(sm.phase(), LoginPhase::LoginComplete);
        assert_eq!(sm.username(), Some("Steve"));
        Ok(())
    }

    #[test]
    fn online_mode_login_flow() -> Result<(), Box<dyn std::error::Error>> {
        let config = LoginConfig {
            online_mode: true,
            compression_threshold: -1,
            max_frame_length: TEST_MAX_FRAME,
            offline_uuid: offline_uuid(),
        };
        let mut sm = LoginStateMachine::new(config);

        // Step 1: Login Start -> Encryption Request
        let wire = encode_login_start_packet("Alex", offline_uuid())?;
        let mut input = wire.as_slice();
        let result = sm.handle_login_start(&mut input)?;
        match result {
            LoginStepResult::Continue { phase, outgoing } => {
                assert_eq!(phase, LoginPhase::AwaitingEncryptionResponse);
                assert_eq!(outgoing.len(), 1); // Encryption Request
            }
            _ => panic!("expected Continue with Encryption Request"),
        }

        // Step 2: Encryption Response -> Login Success
        let wire = encode_encryption_response_packet()?;
        let mut input = wire.as_slice();
        let result = sm.handle_encryption_response(&mut input)?;
        match result {
            LoginStepResult::Success { new_state, .. } => {
                assert_eq!(new_state, ProtocolState::Play);
            }
            _ => panic!("expected Success"),
        }
        assert_eq!(sm.phase(), LoginPhase::LoginComplete);
        assert_eq!(sm.username(), Some("Alex"));
        Ok(())
    }

    #[test]
    fn disconnect_during_login() -> Result<(), Box<dyn std::error::Error>> {
        let config = LoginConfig {
            online_mode: false,
            compression_threshold: -1,
            max_frame_length: TEST_MAX_FRAME,
            offline_uuid: offline_uuid(),
        };
        let mut sm = LoginStateMachine::new(config);

        let result = sm.disconnect("Server is full")?;
        match result {
            LoginStepResult::Disconnect { new_state, .. } => {
                assert_eq!(new_state, ProtocolState::Closed);
            }
            _ => panic!("expected Disconnect"),
        }
        assert_eq!(sm.phase(), LoginPhase::Disconnected);
        Ok(())
    }

    #[test]
    fn repeated_login_start_is_rejected() -> Result<(), Box<dyn std::error::Error>> {
        let config = LoginConfig {
            online_mode: true,
            compression_threshold: -1,
            max_frame_length: TEST_MAX_FRAME,
            offline_uuid: offline_uuid(),
        };
        let mut sm = LoginStateMachine::new(config);

        let wire = encode_login_start_packet("Steve", offline_uuid())?;
        let mut input = wire.as_slice();
        let _ = sm.handle_login_start(&mut input)?;

        // Second Login Start should be rejected
        let wire = encode_login_start_packet("Steve", offline_uuid())?;
        let mut input = wire.as_slice();
        let result = sm.handle_login_start(&mut input);
        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn encryption_response_in_awaiting_login_start_is_rejected()
    -> Result<(), Box<dyn std::error::Error>> {
        let config = LoginConfig {
            online_mode: true,
            compression_threshold: -1,
            max_frame_length: TEST_MAX_FRAME,
            offline_uuid: offline_uuid(),
        };
        let mut sm = LoginStateMachine::new(config);

        let wire = encode_encryption_response_packet()?;
        let mut input = wire.as_slice();
        let result = sm.handle_encryption_response(&mut input);
        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn terminal_state_rejects_further_packets() -> Result<(), Box<dyn std::error::Error>> {
        let config = LoginConfig {
            online_mode: false,
            compression_threshold: -1,
            max_frame_length: TEST_MAX_FRAME,
            offline_uuid: offline_uuid(),
        };
        let mut sm = LoginStateMachine::new(config);

        // Complete login
        let wire = encode_login_start_packet("Steve", offline_uuid())?;
        let mut input = wire.as_slice();
        let _ = sm.handle_login_start(&mut input)?;

        // Further packets should be rejected
        let wire = encode_login_start_packet("Steve", offline_uuid())?;
        let mut input = wire.as_slice();
        let result = sm.handle_login_start(&mut input);
        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn incomplete_login_start_returns_continue() -> Result<(), Box<dyn std::error::Error>> {
        let config = LoginConfig {
            online_mode: false,
            compression_threshold: -1,
            max_frame_length: TEST_MAX_FRAME,
            offline_uuid: offline_uuid(),
        };
        let mut sm = LoginStateMachine::new(config);

        let wire = encode_login_start_packet("Steve", offline_uuid())?;
        let mut input = &wire[..3]; // truncated
        let result = sm.handle_login_start(&mut input)?;
        match result {
            LoginStepResult::Continue { phase, outgoing } => {
                assert_eq!(phase, LoginPhase::AwaitingLoginStart);
                assert!(outgoing.is_empty());
            }
            _ => panic!("expected Continue with no outgoing"),
        }
        Ok(())
    }
}
