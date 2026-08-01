//! Black-box TCP client for the protocol 763 Play exchange.
//!
//! Connects to a configurable host and port, performs the full login (offline
//! mode) -> play exchange, and returns a semantic play snapshot. The oracle
//! path and Java executable are supplied by the caller at runtime; this crate
//! never references or commits them.

use std::fmt;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use rustbound_protocol::framing::{PROTOCOL_MAX_FRAME_LENGTH, decode_frame, encode_frame};
use rustbound_protocol::handshake::{HandshakePacket, encode_handshake};
use rustbound_protocol::login::{
    LoginDecodeOutcome, LoginPacket, LoginStart, decode_login_success, encode_login_start,
};
use rustbound_protocol::play::{PlayDecodeOutcome, PlayError, PlayPacket, decode_join_game};
use rustbound_protocol::primitives::Uuid;
use rustbound_protocol::state::NextState;

/// An error encountered while running a play conformance probe.
#[derive(Debug)]
pub enum PlayClientError {
    /// TCP connection failed.
    Connect(std::io::Error),
    /// A read or write I/O operation failed.
    Io(std::io::Error),
    /// The remote end closed the connection prematurely.
    PrematureEof,
    /// The operation timed out.
    Timeout,
    /// A protocol-level error occurred during the exchange.
    Protocol(String),
}

impl fmt::Display for PlayClientError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Connect(error) => write!(formatter, "connection failed: {error}"),
            Self::Io(error) => write!(formatter, "I/O error: {error}"),
            Self::PrematureEof => formatter.write_str("remote closed connection prematurely"),
            Self::Timeout => formatter.write_str("operation timed out"),
            Self::Protocol(message) => write!(formatter, "protocol error: {message}"),
        }
    }
}

impl std::error::Error for PlayClientError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Connect(error) | Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<rustbound_protocol::framing::FramingError> for PlayClientError {
    fn from(error: rustbound_protocol::framing::FramingError) -> Self {
        Self::Protocol(format!("framing error: {error}"))
    }
}

impl From<rustbound_protocol::login::LoginError> for PlayClientError {
    fn from(error: rustbound_protocol::login::LoginError) -> Self {
        Self::Protocol(format!("login error: {error}"))
    }
}

impl From<PlayError> for PlayClientError {
    fn from(error: PlayError) -> Self {
        Self::Protocol(format!("play error: {error}"))
    }
}

/// The semantic outcome of a play conformance probe.
#[derive(Debug, Clone, PartialEq)]
pub struct PlaySnapshot {
    /// The player's entity ID from Join Game.
    pub entity_id: i32,
    /// The player's gamemode from Join Game.
    pub gamemode: u8,
    /// The dimension name from Join Game.
    pub dimension_name: String,
    /// The hashed seed from Join Game.
    pub hashed_seed: i64,
    /// Whether the world is hardcore.
    pub is_hardcore: bool,
    /// Whether the world is flat.
    pub is_flat: bool,
    /// The player's UUID from Login Success.
    pub uuid: Uuid,
    /// The player's username from Login Success.
    pub username: String,
}

/// Configuration for a play conformance probe.
#[derive(Debug, Clone)]
pub struct PlayProbeConfig {
    /// The server host.
    pub host: String,
    /// The server port.
    pub port: u16,
    /// The username to use for Login Start.
    pub username: String,
    /// The UUID to use for Login Start.
    pub uuid: Uuid,
    /// Connection timeout.
    pub connect_timeout: Duration,
    /// Read timeout.
    pub read_timeout: Duration,
}

/// Runs a play conformance probe in offline mode.
///
/// This performs:
/// 1. TCP connect
/// 2. Handshake with next_state=Login
/// 3. Login Start
/// 4. Wait for Login Success
/// 5. Wait for Join Game
/// 6. Record the play snapshot
pub fn run_play_probe(config: &PlayProbeConfig) -> Result<PlaySnapshot, PlayClientError> {
    let address = format!("{}:{}", config.host, config.port);
    let mut stream = TcpStream::connect_timeout(
        &address.parse().map_err(|e: std::net::AddrParseError| {
            PlayClientError::Protocol(format!("invalid address: {e}"))
        })?,
        config.connect_timeout,
    )
    .map_err(PlayClientError::Connect)?;

    stream
        .set_read_timeout(Some(config.read_timeout))
        .map_err(PlayClientError::Io)?;
    stream
        .set_write_timeout(Some(config.read_timeout))
        .map_err(PlayClientError::Io)?;

    // Send Handshake (next_state = Login)
    let handshake = HandshakePacket {
        protocol_version: rustbound_protocol::PROTOCOL_VERSION as i32,
        server_address: config.host.clone(),
        port: config.port,
        next_state: NextState::Login,
    };
    let mut handshake_wire = Vec::new();
    encode_handshake(&handshake, PROTOCOL_MAX_FRAME_LENGTH, &mut handshake_wire)
        .map_err(|e| PlayClientError::Protocol(e.to_string()))?;
    stream
        .write_all(&handshake_wire)
        .map_err(PlayClientError::Io)?;

    // Send Login Start
    let login_start = LoginStart {
        username: config.username.clone(),
        uuid: config.uuid,
    };
    let mut start_wire = Vec::new();
    encode_login_start(&login_start, PROTOCOL_MAX_FRAME_LENGTH, &mut start_wire)?;
    stream.write_all(&start_wire).map_err(PlayClientError::Io)?;

    // Read frames until we get Login Success
    let mut buffer = Vec::new();
    let mut login_success_info: Option<(Uuid, String)> = None;

    loop {
        let mut chunk = [0u8; 4096];
        let n = stream.read(&mut chunk).map_err(|error| {
            if error.kind() == std::io::ErrorKind::WouldBlock
                || error.kind() == std::io::ErrorKind::TimedOut
            {
                PlayClientError::Timeout
            } else {
                PlayClientError::Io(error)
            }
        })?;

        if n == 0 {
            return Err(PlayClientError::PrematureEof);
        }

        buffer.extend_from_slice(&chunk[..n]);

        // Try to decode frames
        loop {
            let mut input = buffer.as_slice();
            let (packet_id, payload) = match decode_frame(&mut input, PROTOCOL_MAX_FRAME_LENGTH) {
                Ok(rustbound_protocol::framing::DecodeOutcome::Complete(frame)) => {
                    (frame.packet_id, frame.payload.to_vec())
                }
                Ok(rustbound_protocol::framing::DecodeOutcome::Incomplete) => break,
                Err(error) => return Err(PlayClientError::from(error)),
            };

            let consumed = buffer.len() - input.len();
            buffer.drain(..consumed);

            // In Login state, packet IDs:
            // 0x00 = Login Disconnect, 0x02 = Login Success, 0x03 = Set Compression
            match packet_id {
                0x02 => {
                    // Login Success
                    let mut reconstructed = Vec::new();
                    encode_frame(
                        0x02,
                        &payload,
                        PROTOCOL_MAX_FRAME_LENGTH,
                        &mut reconstructed,
                    )?;
                    let mut recon_input = reconstructed.as_slice();
                    match decode_login_success(&mut recon_input, PROTOCOL_MAX_FRAME_LENGTH)? {
                        LoginDecodeOutcome::Complete(LoginPacket::LoginSuccess(success)) => {
                            login_success_info = Some((success.uuid, success.username));
                        }
                        _ => {
                            return Err(PlayClientError::Protocol(
                                "expected Login Success".to_string(),
                            ));
                        }
                    }
                }
                0x03 => {
                    // Set Compression - skip for now
                }
                0x00 => {
                    // Login Disconnect
                    return Err(PlayClientError::Protocol(
                        "server sent Login Disconnect".to_string(),
                    ));
                }
                _ => {
                    return Err(PlayClientError::Protocol(format!(
                        "unexpected packet ID 0x{packet_id:02x} during login"
                    )));
                }
            }

            if login_success_info.is_some() {
                break;
            }
        }

        if login_success_info.is_some() {
            break;
        }
    }

    let (uuid, username) = login_success_info
        .ok_or_else(|| PlayClientError::Protocol("did not receive Login Success".to_string()))?;

    // Now read frames until we get Join Game (Play 0x28)
    buffer.clear();

    loop {
        let mut chunk = [0u8; 4096];
        let n = stream.read(&mut chunk).map_err(|error| {
            if error.kind() == std::io::ErrorKind::WouldBlock
                || error.kind() == std::io::ErrorKind::TimedOut
            {
                PlayClientError::Timeout
            } else {
                PlayClientError::Io(error)
            }
        })?;

        if n == 0 {
            return Err(PlayClientError::PrematureEof);
        }

        buffer.extend_from_slice(&chunk[..n]);

        // Try to decode frames
        loop {
            let mut input = buffer.as_slice();
            let (packet_id, payload) = match decode_frame(&mut input, PROTOCOL_MAX_FRAME_LENGTH) {
                Ok(rustbound_protocol::framing::DecodeOutcome::Complete(frame)) => {
                    (frame.packet_id, frame.payload.to_vec())
                }
                Ok(rustbound_protocol::framing::DecodeOutcome::Incomplete) => break,
                Err(error) => return Err(PlayClientError::from(error)),
            };

            let consumed = buffer.len() - input.len();
            buffer.drain(..consumed);

            // In Play state, packet ID 0x28 = Join Game
            match packet_id {
                0x28 => {
                    let mut reconstructed = Vec::new();
                    encode_frame(
                        0x28,
                        &payload,
                        PROTOCOL_MAX_FRAME_LENGTH,
                        &mut reconstructed,
                    )?;
                    let mut recon_input = reconstructed.as_slice();
                    match decode_join_game(&mut recon_input, PROTOCOL_MAX_FRAME_LENGTH)? {
                        PlayDecodeOutcome::Complete(PlayPacket::JoinGame(join)) => {
                            return Ok(PlaySnapshot {
                                entity_id: join.entity_id,
                                gamemode: join.gamemode.to_wire(),
                                dimension_name: join.dimension_name,
                                hashed_seed: join.hashed_seed,
                                is_hardcore: join.is_hardcore,
                                is_flat: join.is_flat,
                                uuid,
                                username,
                            });
                        }
                        _ => {
                            return Err(PlayClientError::Protocol(
                                "expected Join Game".to_string(),
                            ));
                        }
                    }
                }
                _ => {
                    // Skip other play packets for now
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{PlayClientError, PlaySnapshot};

    #[test]
    fn play_client_error_display_is_informative() {
        let error = PlayClientError::PrematureEof;
        assert!(format!("{error}").contains("prematurely"));
    }

    #[test]
    fn play_snapshot_equality_works() {
        let uuid = rustbound_protocol::primitives::Uuid::new(0, 0);
        let a = PlaySnapshot {
            entity_id: 42,
            gamemode: 0,
            dimension_name: "minecraft:overworld".to_string(),
            hashed_seed: 0,
            is_hardcore: false,
            is_flat: false,
            uuid,
            username: "Steve".to_string(),
        };
        let b = PlaySnapshot {
            entity_id: 42,
            gamemode: 0,
            dimension_name: "minecraft:overworld".to_string(),
            hashed_seed: 0,
            is_hardcore: false,
            is_flat: false,
            uuid,
            username: "Steve".to_string(),
        };
        assert_eq!(a, b);
    }
}
