//! Connection handler and state router for the Rustbound server.
//!
//! Each accepted connection is processed by a `handle_connection` function
//! that reads packets from the TCP stream, routes them to the correct state
//! codec (Handshaking, Status, Login, Play), and writes responses. The
//! handler runs on its own thread.

use std::fmt;
use std::io::{Read, Write};
use std::net::TcpStream;

use rustbound_protocol::framing::PROTOCOL_MAX_FRAME_LENGTH;
use rustbound_protocol::handshake::{HandshakeError, HandshakePacket, decode_handshake};
use rustbound_protocol::login::{
    LoginDecodeOutcome, LoginError, LoginPacket, LoginStart, decode_login_start,
    encode_login_success,
};
use rustbound_protocol::state::ProtocolState;
use rustbound_protocol::status::{
    StatusError, StatusResponse, decode_ping_request, decode_status_request, encode_pong_response,
    encode_status_response,
};

/// An error encountered while handling a connection.
#[derive(Debug)]
pub enum ConnectionError {
    /// An I/O error occurred.
    Io(std::io::Error),
    /// The remote end closed the connection.
    Disconnected,
    /// A framing error occurred.
    Framing(rustbound_protocol::framing::FramingError),
    /// A handshake error occurred.
    Handshake(HandshakeError),
    /// A status protocol error occurred.
    Status(StatusError),
    /// A login protocol error occurred.
    Login(LoginError),
    /// An unexpected packet was received in the current state.
    UnexpectedPacket {
        /// The current protocol state.
        state: ProtocolState,
        /// The received packet ID.
        packet_id: i32,
    },
}

impl fmt::Display for ConnectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "I/O error: {error}"),
            Self::Disconnected => formatter.write_str("remote disconnected"),
            Self::Framing(error) => write!(formatter, "framing error: {error}"),
            Self::Handshake(error) => write!(formatter, "handshake error: {error}"),
            Self::Status(error) => write!(formatter, "status error: {error}"),
            Self::Login(error) => write!(formatter, "login error: {error}"),
            Self::UnexpectedPacket { state, packet_id } => {
                write!(
                    formatter,
                    "unexpected packet 0x{packet_id:02x} in {state} state"
                )
            }
        }
    }
}

impl std::error::Error for ConnectionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Framing(error) => Some(error),
            Self::Handshake(error) => Some(error),
            Self::Status(error) => Some(error),
            Self::Login(error) => Some(error),
            _ => None,
        }
    }
}

impl From<std::io::Error> for ConnectionError {
    fn from(error: std::io::Error) -> Self {
        if error.kind() == std::io::ErrorKind::UnexpectedEof {
            Self::Disconnected
        } else {
            Self::Io(error)
        }
    }
}

impl From<rustbound_protocol::framing::FramingError> for ConnectionError {
    fn from(error: rustbound_protocol::framing::FramingError) -> Self {
        Self::Framing(error)
    }
}

impl From<HandshakeError> for ConnectionError {
    fn from(error: HandshakeError) -> Self {
        Self::Handshake(error)
    }
}

impl From<StatusError> for ConnectionError {
    fn from(error: StatusError) -> Self {
        Self::Status(error)
    }
}

impl From<LoginError> for ConnectionError {
    fn from(error: LoginError) -> Self {
        Self::Login(error)
    }
}

/// Configuration for the connection handler.
#[derive(Debug, Clone)]
pub struct ConnectionConfig {
    /// The server's status response (for Status state).
    pub status_response: StatusResponse,
    /// Whether the server is in online mode.
    pub online_mode: bool,
    /// The maximum frame length.
    pub max_frame_length: usize,
}

impl Default for ConnectionConfig {
    fn default() -> Self {
        Self {
            status_response: default_status_response(),
            online_mode: false,
            max_frame_length: PROTOCOL_MAX_FRAME_LENGTH,
        }
    }
}

/// The result of handling a login packet.
enum LoginHandleResult {
    /// Login succeeded, transition to Play.
    Success,
    /// The client was disconnected.
    Disconnect,
}

/// Handles a single TCP connection through the full protocol lifecycle.
///
/// This function reads packets, routes them based on the current protocol
/// state, and writes responses. It returns when the connection is closed
/// or an error occurs.
pub fn handle_connection(
    mut stream: TcpStream,
    config: &ConnectionConfig,
) -> Result<(), ConnectionError> {
    let mut state = ProtocolState::Handshaking;
    let mut read_buffer = Vec::with_capacity(4096);

    loop {
        // Try to decode a packet from the buffer based on current state.
        // The protocol decode functions handle framing internally and
        // return Incomplete if there isn't enough data yet.
        let decode_result = try_decode_packet(&mut read_buffer, state, config.max_frame_length);

        match decode_result {
            DecodeResult::Complete(packet) => {
                // Route the packet based on the current state
                match state {
                    ProtocolState::Handshaking => {
                        if let DecodedPacket::Handshake(handshake) = packet {
                            state = handshake.next_state.target_state();
                        } else {
                            return Err(ConnectionError::UnexpectedPacket {
                                state,
                                packet_id: -1,
                            });
                        }
                    }
                    ProtocolState::Status => {
                        state = handle_status_packet(
                            packet,
                            &config.status_response,
                            config.max_frame_length,
                            &mut stream,
                        )?;
                        if state == ProtocolState::Closed {
                            return Ok(());
                        }
                    }
                    ProtocolState::Login => {
                        let result = handle_login_packet(packet, config, &mut stream)?;
                        match result {
                            LoginHandleResult::Success => {
                                // Transition to Play (actual play handling in Issue #41+)
                                return Ok(());
                            }
                            LoginHandleResult::Disconnect => {
                                return Ok(());
                            }
                        }
                    }
                    ProtocolState::Play => {
                        // Play handling will be implemented in Issue #41+
                        return Ok(());
                    }
                    ProtocolState::Closed => {
                        return Ok(());
                    }
                }
            }
            DecodeResult::Incomplete => {
                // Need more data - read from the stream
                let mut chunk = [0u8; 4096];
                let n = stream.read(&mut chunk)?;
                if n == 0 {
                    return Err(ConnectionError::Disconnected);
                }
                read_buffer.extend_from_slice(&chunk[..n]);
            }
            DecodeResult::Error(error) => {
                return Err(error);
            }
        }
    }
}

/// A decoded packet from any state.
enum DecodedPacket {
    Handshake(HandshakePacket),
    StatusRequest,
    Ping(i64),
    LoginStart(LoginStart),
}

/// The result of attempting to decode a packet.
enum DecodeResult {
    /// A complete packet was decoded.
    Complete(DecodedPacket),
    /// Not enough data yet.
    Incomplete,
    /// An error occurred.
    Error(ConnectionError),
}

/// Attempts to decode a packet from the buffer based on the current state.
///
/// On `Incomplete`, the buffer is left unchanged. On `Complete`, the
/// consumed bytes are drained from the buffer.
fn try_decode_packet(
    buffer: &mut Vec<u8>,
    state: ProtocolState,
    max_frame_length: usize,
) -> DecodeResult {
    let source = buffer.as_slice();
    let mut input = source;

    let result = match state {
        ProtocolState::Handshaking => match decode_handshake(&mut input, max_frame_length) {
            Ok(packet) => Ok(DecodedPacket::Handshake(packet)),
            Err(HandshakeError::Incomplete) => Err(None),
            Err(error) => Err(Some(ConnectionError::Handshake(error))),
        },
        ProtocolState::Status => {
            // Try status request (packet ID 0x00) first
            match decode_status_request(&mut input, max_frame_length) {
                Ok(Some(())) => Ok(DecodedPacket::StatusRequest),
                Ok(None) => Err(None),
                Err(StatusError::WrongPacketId { received: 1, .. }) => {
                    // It's a ping request - retry with the original buffer
                    input = source;
                    match decode_ping_request(&mut input, max_frame_length) {
                        Ok(Some(payload)) => Ok(DecodedPacket::Ping(payload)),
                        Ok(None) => Err(None),
                        Err(error) => Err(Some(ConnectionError::Status(error))),
                    }
                }
                Err(error) => Err(Some(ConnectionError::Status(error))),
            }
        }
        ProtocolState::Login => match decode_login_start(&mut input, max_frame_length) {
            Ok(LoginDecodeOutcome::Complete(LoginPacket::LoginStart(start))) => {
                Ok(DecodedPacket::LoginStart(start))
            }
            Ok(LoginDecodeOutcome::Incomplete) => Err(None),
            Ok(_) => Err(Some(ConnectionError::UnexpectedPacket {
                state,
                packet_id: -1,
            })),
            Err(error) => {
                if let LoginError::WrongPacketId { received, .. } = &error {
                    Err(Some(ConnectionError::UnexpectedPacket {
                        state,
                        packet_id: *received,
                    }))
                } else {
                    Err(Some(ConnectionError::Login(error)))
                }
            }
        },
        _ => {
            return DecodeResult::Error(ConnectionError::UnexpectedPacket {
                state,
                packet_id: -1,
            });
        }
    };

    match result {
        Ok(packet) => {
            // Drain consumed bytes
            let consumed = source.len() - input.len();
            buffer.drain(..consumed);
            DecodeResult::Complete(packet)
        }
        Err(None) => DecodeResult::Incomplete,
        Err(Some(error)) => DecodeResult::Error(error),
    }
}

/// Handles a status-state packet.
///
/// Returns the new protocol state. `Closed` means the connection should be
/// closed.
fn handle_status_packet(
    packet: DecodedPacket,
    status_response: &StatusResponse,
    max_frame_length: usize,
    stream: &mut TcpStream,
) -> Result<ProtocolState, ConnectionError> {
    match packet {
        DecodedPacket::StatusRequest => {
            // Send status response
            let mut wire = Vec::new();
            encode_status_response(status_response, max_frame_length, &mut wire)?;
            stream.write_all(&wire)?;
            Ok(ProtocolState::Status)
        }
        DecodedPacket::Ping(payload) => {
            // Send pong response
            let mut wire = Vec::new();
            encode_pong_response(payload, max_frame_length, &mut wire)?;
            stream.write_all(&wire)?;
            Ok(ProtocolState::Closed)
        }
        _ => Err(ConnectionError::UnexpectedPacket {
            state: ProtocolState::Status,
            packet_id: -1,
        }),
    }
}

/// Handles a login-state packet.
fn handle_login_packet(
    packet: DecodedPacket,
    config: &ConnectionConfig,
    stream: &mut TcpStream,
) -> Result<LoginHandleResult, ConnectionError> {
    match packet {
        DecodedPacket::LoginStart(start) => {
            if config.online_mode {
                // Online mode would send Encryption Request here
                // For now, just disconnect
                return Ok(LoginHandleResult::Disconnect);
            }

            // Offline mode: send Login Success
            let success = rustbound_protocol::login::LoginSuccess {
                uuid: start.uuid,
                username: start.username,
                properties: Vec::new(),
            };
            let mut wire = Vec::new();
            encode_login_success(&success, config.max_frame_length, &mut wire)?;
            stream.write_all(&wire)?;
            Ok(LoginHandleResult::Success)
        }
        _ => Err(ConnectionError::UnexpectedPacket {
            state: ProtocolState::Login,
            packet_id: -1,
        }),
    }
}

/// Creates a default status response for testing.
pub fn default_status_response() -> StatusResponse {
    StatusResponse {
        version: rustbound_protocol::status::StatusVersion {
            name: "1.20.1".to_string(),
            protocol: 763,
        },
        players: rustbound_protocol::status::StatusPlayers {
            max: 20,
            online: 0,
            sample: Some(Vec::new()),
        },
        description: rustbound_protocol::status::StatusDescription {
            text: "A Rust Minecraft Server".to_string(),
        },
        favicon: None,
    }
}

#[cfg(test)]
mod tests {
    use super::{ConnectionConfig, default_status_response};
    use rustbound_protocol::framing::{decode_frame, encode_frame};
    use rustbound_protocol::handshake::HandshakePacket;
    use rustbound_protocol::primitives::{Uuid, encode_i64};
    use rustbound_protocol::state::NextState;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::time::Duration;

    fn find_free_port() -> Result<u16, Box<dyn std::error::Error>> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        Ok(listener.local_addr()?.port())
    }

    fn encode_handshake_packet(next_state: NextState) -> Vec<u8> {
        let handshake = HandshakePacket {
            protocol_version: 763,
            server_address: "127.0.0.1".to_string(),
            port: 25565,
            next_state,
        };
        let mut wire = Vec::new();
        let _ = rustbound_protocol::handshake::encode_handshake(&handshake, 65536, &mut wire);
        wire
    }

    fn encode_status_request_packet() -> Vec<u8> {
        let body = Vec::new();
        let mut wire = Vec::new();
        let _ = encode_frame(0x00, &body, 65536, &mut wire);
        wire
    }

    fn encode_ping_request_packet(payload: i64) -> Vec<u8> {
        let mut body = Vec::new();
        encode_i64(payload, &mut body);
        let mut wire = Vec::new();
        let _ = encode_frame(0x01, &body, 65536, &mut wire);
        wire
    }

    #[test]
    fn connection_handler_status_exchange() -> Result<(), Box<dyn std::error::Error>> {
        let port = find_free_port()?;
        let listener = TcpListener::bind(format!("127.0.0.1:{port}"))?;

        let config = ConnectionConfig {
            status_response: default_status_response(),
            online_mode: false,
            max_frame_length: 65536,
        };

        let handler_thread = std::thread::spawn(move || {
            if let Ok((stream, _)) = listener.accept() {
                let _ = super::handle_connection(stream, &config);
            }
        });

        std::thread::sleep(Duration::from_millis(50));

        let mut stream = TcpStream::connect(format!("127.0.0.1:{port}"))?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        stream.set_write_timeout(Some(Duration::from_secs(5)))?;

        // Send handshake (next_state = Status)
        stream.write_all(&encode_handshake_packet(NextState::Status))?;

        // Send status request
        stream.write_all(&encode_status_request_packet())?;

        // Read status response
        let mut buf = vec![0u8; 4096];
        let n = stream.read(&mut buf)?;
        let mut input = &buf[..n];
        let frame = match decode_frame(&mut input, 65536)? {
            rustbound_protocol::framing::DecodeOutcome::Complete(f) => f,
            _ => panic!("expected complete frame"),
        };
        assert_eq!(frame.packet_id, 0x00);
        assert!(!frame.payload.is_empty());

        // Send ping request
        stream.write_all(&encode_ping_request_packet(42))?;

        // Read pong response
        let n = stream.read(&mut buf)?;
        let mut input = &buf[..n];
        let frame = match decode_frame(&mut input, 65536)? {
            rustbound_protocol::framing::DecodeOutcome::Complete(f) => f,
            _ => panic!("expected complete frame"),
        };
        assert_eq!(frame.packet_id, 0x01);

        let _ = handler_thread.join();
        Ok(())
    }

    #[test]
    fn connection_handler_login_offline_mode() -> Result<(), Box<dyn std::error::Error>> {
        let port = find_free_port()?;
        let listener = TcpListener::bind(format!("127.0.0.1:{port}"))?;

        let config = ConnectionConfig {
            status_response: default_status_response(),
            online_mode: false,
            max_frame_length: 65536,
        };

        let handler_thread = std::thread::spawn(move || {
            if let Ok((stream, _)) = listener.accept() {
                let _ = super::handle_connection(stream, &config);
            }
        });

        std::thread::sleep(Duration::from_millis(50));

        let mut stream = TcpStream::connect(format!("127.0.0.1:{port}"))?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        stream.set_write_timeout(Some(Duration::from_secs(5)))?;

        // Send handshake (next_state = Login)
        stream.write_all(&encode_handshake_packet(NextState::Login))?;

        // Send login start
        let login_start = rustbound_protocol::login::LoginStart {
            username: "TestPlayer".to_string(),
            uuid: Uuid::new(0, 0),
        };
        let mut start_wire = Vec::new();
        rustbound_protocol::login::encode_login_start(&login_start, 65536, &mut start_wire)?;
        stream.write_all(&start_wire)?;

        // Read login success
        let mut buf = vec![0u8; 4096];
        let n = stream.read(&mut buf)?;
        let mut input = &buf[..n];
        let frame = match decode_frame(&mut input, 65536)? {
            rustbound_protocol::framing::DecodeOutcome::Complete(f) => f,
            _ => panic!("expected complete frame"),
        };
        assert_eq!(frame.packet_id, 0x02); // Login Success

        let _ = handler_thread.join();
        Ok(())
    }
}
