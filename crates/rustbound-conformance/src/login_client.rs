//! Black-box TCP client for the protocol 763 Login exchange.
//!
//! Connects to a configurable host and port, performs the Handshake (Login) ->
//! Login Start exchange in offline mode, and returns a semantic login outcome.
//! The oracle path and Java executable are supplied by the caller at runtime;
//! this crate never references or commits them.

use std::fmt;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use rustbound_protocol::framing::{PROTOCOL_MAX_FRAME_LENGTH, decode_frame, encode_frame};
use rustbound_protocol::handshake::{HandshakePacket, encode_handshake};
use rustbound_protocol::login::{
    LoginDecodeOutcome, LoginDisconnect, LoginPacket, LoginStart, decode_login_disconnect,
    decode_login_success, encode_login_start,
};
use rustbound_protocol::primitives::Uuid;
use rustbound_protocol::state::NextState;

/// An error encountered while running a login conformance probe.
#[derive(Debug)]
pub enum LoginClientError {
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

impl fmt::Display for LoginClientError {
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

impl std::error::Error for LoginClientError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Connect(error) | Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<rustbound_protocol::framing::FramingError> for LoginClientError {
    fn from(error: rustbound_protocol::framing::FramingError) -> Self {
        Self::Protocol(format!("framing error: {error}"))
    }
}

impl From<rustbound_protocol::login::LoginError> for LoginClientError {
    fn from(error: rustbound_protocol::login::LoginError) -> Self {
        Self::Protocol(format!("login error: {error}"))
    }
}

impl From<rustbound_protocol::primitives::CodecError> for LoginClientError {
    fn from(error: rustbound_protocol::primitives::CodecError) -> Self {
        Self::Protocol(format!("codec error: {error}"))
    }
}

/// The semantic outcome of a login conformance probe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginOutcome {
    /// Login succeeded.
    Success {
        /// The player's UUID.
        uuid: Uuid,
        /// The player's username.
        username: String,
        /// The number of properties (signatures are not compared).
        property_count: usize,
    },
    /// The server sent a Login Disconnect.
    Disconnect {
        /// The disconnect reason (JSON string).
        reason: String,
    },
}

/// Configuration for a login conformance probe.
#[derive(Debug, Clone)]
pub struct LoginProbeConfig {
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

/// Runs a login conformance probe in offline mode.
///
/// This performs:
/// 1. TCP connect
/// 2. Handshake with next_state=Login
/// 3. Login Start
/// 4. Wait for Login Success or Login Disconnect
pub fn run_login_probe(config: &LoginProbeConfig) -> Result<LoginOutcome, LoginClientError> {
    let address = format!("{}:{}", config.host, config.port);
    let mut stream = TcpStream::connect_timeout(
        &address.parse().map_err(|e: std::net::AddrParseError| {
            LoginClientError::Protocol(format!("invalid address: {e}"))
        })?,
        config.connect_timeout,
    )
    .map_err(LoginClientError::Connect)?;

    stream
        .set_read_timeout(Some(config.read_timeout))
        .map_err(LoginClientError::Io)?;
    stream
        .set_write_timeout(Some(config.read_timeout))
        .map_err(LoginClientError::Io)?;

    // Send Handshake (next_state = Login)
    let handshake = HandshakePacket {
        protocol_version: rustbound_protocol::PROTOCOL_VERSION as i32,
        server_address: config.host.clone(),
        port: config.port,
        next_state: NextState::Login,
    };
    let mut handshake_wire = Vec::new();
    encode_handshake(&handshake, PROTOCOL_MAX_FRAME_LENGTH, &mut handshake_wire)
        .map_err(|error| LoginClientError::Protocol(error.to_string()))?;
    stream
        .write_all(&handshake_wire)
        .map_err(LoginClientError::Io)?;

    // Send Login Start
    let login_start = LoginStart {
        username: config.username.clone(),
        uuid: config.uuid,
    };
    let mut start_wire = Vec::new();
    encode_login_start(&login_start, PROTOCOL_MAX_FRAME_LENGTH, &mut start_wire)?;
    stream
        .write_all(&start_wire)
        .map_err(LoginClientError::Io)?;

    // Read response frames until we get Login Success or Login Disconnect
    let mut buffer = Vec::new();
    loop {
        let mut chunk = [0u8; 4096];
        let n = stream.read(&mut chunk).map_err(|error| {
            if error.kind() == std::io::ErrorKind::WouldBlock
                || error.kind() == std::io::ErrorKind::TimedOut
            {
                LoginClientError::Timeout
            } else {
                LoginClientError::Io(error)
            }
        })?;

        if n == 0 {
            return Err(LoginClientError::PrematureEof);
        }

        buffer.extend_from_slice(&chunk[..n]);

        // Try to decode a frame
        let mut input = buffer.as_slice();
        let (packet_id, payload) = match decode_frame(&mut input, PROTOCOL_MAX_FRAME_LENGTH) {
            Ok(rustbound_protocol::framing::DecodeOutcome::Complete(frame)) => {
                (frame.packet_id, frame.payload.to_vec())
            }
            Ok(rustbound_protocol::framing::DecodeOutcome::Incomplete) => continue,
            Err(error) => return Err(LoginClientError::from(error)),
        };

        // Consume the frame from the buffer
        let consumed = buffer.len() - input.len();
        buffer.drain(..consumed);

        // Dispatch based on packet ID
        match packet_id {
            0x00 => {
                // Login Disconnect (clientbound 0x00)
                let mut reconstructed = Vec::new();
                encode_frame(
                    0x00,
                    &payload,
                    PROTOCOL_MAX_FRAME_LENGTH,
                    &mut reconstructed,
                )?;
                let mut recon_input = reconstructed.as_slice();
                match decode_login_disconnect(&mut recon_input, PROTOCOL_MAX_FRAME_LENGTH)? {
                    LoginDecodeOutcome::Complete(LoginPacket::LoginDisconnect(
                        LoginDisconnect { reason },
                    )) => {
                        return Ok(LoginOutcome::Disconnect { reason });
                    }
                    _ => {
                        return Err(LoginClientError::Protocol(
                            "expected Login Disconnect".to_string(),
                        ));
                    }
                }
            }
            0x02 => {
                // Login Success (clientbound 0x02)
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
                        return Ok(LoginOutcome::Success {
                            uuid: success.uuid,
                            username: success.username,
                            property_count: success.properties.len(),
                        });
                    }
                    _ => {
                        return Err(LoginClientError::Protocol(
                            "expected Login Success".to_string(),
                        ));
                    }
                }
            }
            0x03 => {
                // Set Compression - skip for now (offline mode typically doesn't use it)
                continue;
            }
            other => {
                return Err(LoginClientError::Protocol(format!(
                    "unexpected packet ID 0x{other:02x} during login"
                )));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{LoginClientError, LoginOutcome};
    use crate::snapshot::StatusSnapshot;

    #[test]
    fn login_client_error_display_is_informative() {
        let error = LoginClientError::PrematureEof;
        assert!(format!("{error}").contains("prematurely"));
    }

    #[test]
    fn login_outcome_equality_works() {
        let a = LoginOutcome::Disconnect {
            reason: "test".to_string(),
        };
        let b = LoginOutcome::Disconnect {
            reason: "test".to_string(),
        };
        assert_eq!(a, b);
    }

    #[test]
    fn login_outcome_success_equality_works() {
        let uuid = rustbound_protocol::primitives::Uuid::new(0, 0);
        let a = LoginOutcome::Success {
            uuid,
            username: "Steve".to_string(),
            property_count: 0,
        };
        let b = LoginOutcome::Success {
            uuid,
            username: "Steve".to_string(),
            property_count: 0,
        };
        assert_eq!(a, b);
    }

    // This test ensures the snapshot module is still referenced (suppresses
    // unused import warnings in some configurations).
    #[test]
    fn snapshot_module_still_compiles() {
        let _ = std::marker::PhantomData::<StatusSnapshot>;
    }
}
