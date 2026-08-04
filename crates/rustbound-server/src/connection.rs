//! Connection handler and state router for the Rustbound server.
//!
//! Each accepted connection is processed by a `handle_connection` function
//! that reads packets from the TCP stream, routes them to the correct state
//! codec (Handshaking, Status, Login, Play), and writes responses. The
//! handler runs on its own thread.

use std::fmt;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::sync::mpsc::Sender;
use std::time::Duration;

use rustbound_protocol::compression::{encode_compressed_frame, encode_set_compression};
use rustbound_protocol::framing::{DecodeOutcome, decode_frame};
use rustbound_protocol::handshake::{HandshakeError, HandshakePacket, decode_handshake};
use rustbound_protocol::login::{
    LoginDecodeOutcome, LoginError, LoginPacket, LoginStart, decode_login_start,
};
use rustbound_protocol::login_state_machine::{
    LoginConfig, LoginStateMachine, LoginStateMachineError, LoginStepResult,
};
use rustbound_protocol::primitives::Uuid;
use rustbound_protocol::state::ProtocolState;
use rustbound_protocol::status::{
    StatusError, StatusResponse, decode_ping_request, decode_status_request, encode_pong_response,
    encode_status_response,
};

use crate::session::{EntityIdAllocator, SessionConfig, SessionError, run_play_loop};
use crate::tick::TickMessage;

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
    /// A login state machine error occurred.
    LoginStateMachine(LoginStateMachineError),
    /// A compression error occurred.
    Compression(rustbound_protocol::compression::CompressionError),
    /// A play session error occurred.
    Session(SessionError),
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
            Self::LoginStateMachine(error) => {
                write!(formatter, "login state machine error: {error}")
            }
            Self::Compression(error) => write!(formatter, "compression error: {error}"),
            Self::Session(error) => write!(formatter, "session error: {error}"),
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
            Self::LoginStateMachine(error) => Some(error),
            Self::Compression(error) => Some(error),
            Self::Session(error) => Some(error),
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

impl From<LoginStateMachineError> for ConnectionError {
    fn from(error: LoginStateMachineError) -> Self {
        Self::LoginStateMachine(error)
    }
}

impl From<rustbound_protocol::compression::CompressionError> for ConnectionError {
    fn from(error: rustbound_protocol::compression::CompressionError) -> Self {
        Self::Compression(error)
    }
}

impl From<SessionError> for ConnectionError {
    fn from(error: SessionError) -> Self {
        Self::Session(error)
    }
}

/// Configuration for the connection handler.
#[derive(Debug, Clone)]
pub struct ConnectionConfig {
    /// The server's status response template (for Status state).
    pub status_response: StatusResponse,
    /// Whether the server is in online mode.
    pub online_mode: bool,
    /// The maximum frame length.
    pub max_frame_length: usize,
    /// The network compression threshold (-1 disables, >= 0 enables).
    pub compression_threshold: i32,
    /// Sender to the tick loop for player state messages.
    pub tick_sender: Sender<TickMessage>,
    /// Entity ID allocator for new player sessions.
    pub entity_id_allocator: EntityIdAllocator,
    /// Read timeout for Play-state connections.
    pub play_read_timeout: Duration,
    /// Live player count (incremented on join, decremented on leave).
    pub player_count: Arc<std::sync::atomic::AtomicUsize>,
    /// Maximum players (from config).
    pub max_players: i32,
    /// Default gamemode for new players (0=Survival, 1=Creative).
    pub default_gamemode: u8,
    /// Server view distance (in chunks).
    pub view_distance: i32,
    /// Server simulation distance (in chunks).
    pub simulation_distance: i32,
    /// Server MOTD (message of the day).
    pub motd: String,
    /// Keep Alive timeout: clients that don't respond within this duration are kicked.
    pub keep_alive_timeout: Duration,
    /// Enabled hakoniwa dimensions for Join Game.
    pub enabled_dimensions: crate::hakoniwa::DimensionSet,
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
                        // Build a status response with the live player count
                        let mut response = config.status_response.clone();
                        let online = config
                            .player_count
                            .load(std::sync::atomic::Ordering::Acquire);
                        response.players.online = i32::try_from(online).unwrap_or(i32::MAX);
                        response.players.max = config.max_players;
                        state = handle_status_packet(
                            packet,
                            &response,
                            config.max_frame_length,
                            &mut stream,
                        )?;
                        if state == ProtocolState::Closed {
                            return Ok(());
                        }
                    }
                    ProtocolState::Login => {
                        // Use the LoginStateMachine for the login flow.
                        let login_config = LoginConfig {
                            online_mode: config.online_mode,
                            compression_threshold: config.compression_threshold,
                            max_frame_length: config.max_frame_length,
                            offline_uuid: Uuid::new(0, 0), // placeholder, replaced below
                        };
                        let mut login_sm = LoginStateMachine::new(login_config);

                        // Feed the decoded Login Start into the SM.
                        if let DecodedPacket::LoginStart(start) = packet {
                            // Encode Login Start back to bytes for the SM to decode.
                            let mut start_wire = Vec::new();
                            rustbound_protocol::login::encode_login_start(
                                &start,
                                config.max_frame_length,
                                &mut start_wire,
                            )?;
                            let mut input = start_wire.as_slice();
                            let step_result = login_sm.handle_login_start(&mut input)?;

                            match step_result {
                                LoginStepResult::Success { outgoing, .. } => {
                                    // If compression is enabled, send Set Compression first
                                    if config.compression_threshold >= 0 {
                                        let mut comp_wire = Vec::new();
                                        encode_set_compression(
                                            config.compression_threshold,
                                            config.max_frame_length,
                                            &mut comp_wire,
                                        )?;
                                        stream.write_all(&comp_wire)?;

                                        // Re-encode Login Success as compressed frame
                                        let mut frame_input = outgoing.as_slice();
                                        let frame = match decode_frame(
                                            &mut frame_input,
                                            config.max_frame_length,
                                        )? {
                                            DecodeOutcome::Complete(f) => f,
                                            DecodeOutcome::Incomplete => {
                                                return Err(ConnectionError::Framing(
                                                    rustbound_protocol::framing::FramingError::ZeroFrameLength,
                                                ));
                                            }
                                        };
                                        let mut compressed_out = Vec::new();
                                        encode_compressed_frame(
                                            frame.packet_id,
                                            frame.payload,
                                            config.compression_threshold,
                                            config.max_frame_length,
                                            &mut compressed_out,
                                        )?;
                                        stream.write_all(&compressed_out)?;
                                    } else {
                                        // Send Login Success uncompressed
                                        stream.write_all(&outgoing)?;
                                    }

                                    let username =
                                        login_sm.username().unwrap_or("Player").to_string();
                                    let uuid =
                                        crate::offline_uuid::offline_uuid_from_username(&username);

                                    // Transition to Play - enter the play loop
                                    eprintln!(
                                        "login ok for '{username}', entering play (compression={})",
                                        config.compression_threshold
                                    );
                                    match run_play_loop(
                                        &mut stream,
                                        &SessionConfig {
                                            uuid,
                                            username: username.clone(),
                                            gamemode: config.default_gamemode,
                                            max_frame_length: config.max_frame_length,
                                            read_timeout: config.play_read_timeout,
                                            compression_threshold: config.compression_threshold,
                                            view_distance: config.view_distance,
                                            simulation_distance: config.simulation_distance,
                                            max_players: config.max_players,
                                            keep_alive_timeout: config.keep_alive_timeout,
                                            enabled_dimensions: config.enabled_dimensions,
                                        },
                                        config.entity_id_allocator.allocate(),
                                        config.tick_sender.clone(),
                                    ) {
                                        Ok(()) => {
                                            eprintln!("play loop ended cleanly for '{username}'");
                                            return Ok(());
                                        }
                                        Err(SessionError::Disconnected) => {
                                            // Client closed the TCP connection — often because it
                                            // rejected a clientbound packet while decoding (e.g. NBT).
                                            eprintln!(
                                                "play loop: client '{username}' disconnected remotely"
                                            );
                                            return Ok(());
                                        }
                                        Err(e) => {
                                            eprintln!("play loop error for '{username}': {e}");
                                            return Err(ConnectionError::from(e));
                                        }
                                    }
                                }
                                LoginStepResult::Continue { outgoing, .. } => {
                                    // Online mode: send Encryption Request
                                    for wire in outgoing {
                                        stream.write_all(&wire)?;
                                    }
                                    // For now, online mode is not fully supported
                                    // The SM will wait for Encryption Response
                                    // Fall through to read more data
                                }
                                LoginStepResult::Disconnect { outgoing, .. } => {
                                    stream.write_all(&outgoing)?;
                                    return Ok(());
                                }
                            }
                        } else {
                            return Err(ConnectionError::UnexpectedPacket {
                                state: ProtocolState::Login,
                                packet_id: -1,
                            });
                        }
                    }
                    ProtocolState::Play => {
                        // Play state is handled by run_play_loop above, not here.
                        // If we reach this point, something went wrong.
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
                match stream.read(&mut chunk) {
                    Ok(0) => return Err(ConnectionError::Disconnected),
                    Ok(n) => read_buffer.extend_from_slice(&chunk[..n]),
                    Err(e)
                        if e.kind() == std::io::ErrorKind::WouldBlock
                            || e.kind() == std::io::ErrorKind::TimedOut
                            || e.kind() == std::io::ErrorKind::Interrupted =>
                    {
                        // Transient; retry. Windows non-blocking leftovers
                        // surface as WouldBlock (10035).
                        continue;
                    }
                    Err(e) => return Err(ConnectionError::from(e)),
                }
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

/// Creates a `StatusResponse` from server config values.
///
/// Uses the config's MOTD for the description and `max_players` for the
/// player cap. The online count is 0 (the actual count is injected by
/// the connection handler if needed).
pub fn status_response_from_config(motd: &str, max_players: i32) -> StatusResponse {
    StatusResponse {
        version: rustbound_protocol::status::StatusVersion {
            name: "1.20.1".to_string(),
            protocol: 763,
        },
        players: rustbound_protocol::status::StatusPlayers {
            max: max_players,
            online: 0,
            sample: Some(Vec::new()),
        },
        description: rustbound_protocol::status::StatusDescription {
            text: motd.to_string(),
        },
        favicon: None,
    }
}

#[cfg(test)]
mod tests {
    use super::{ConnectionConfig, default_status_response, status_response_from_config};
    use crate::session::EntityIdAllocator;
    use rustbound_protocol::framing::{decode_frame, encode_frame};
    use rustbound_protocol::handshake::HandshakePacket;
    use rustbound_protocol::primitives::{Uuid, encode_i64};
    use rustbound_protocol::state::NextState;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::Arc;
    use std::sync::mpsc::channel;
    use std::time::Duration;

    fn find_free_port() -> Result<u16, Box<dyn std::error::Error>> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        Ok(listener.local_addr()?.port())
    }

    fn test_connection_config() -> ConnectionConfig {
        let (tx, _rx) = channel::<crate::tick::TickMessage>();
        ConnectionConfig {
            status_response: default_status_response(),
            online_mode: false,
            max_frame_length: 65536,
            compression_threshold: -1, // disabled for basic tests
            tick_sender: tx,
            entity_id_allocator: EntityIdAllocator::new(1),
            play_read_timeout: Duration::from_secs(5),
            player_count: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            max_players: 20,
            default_gamemode: 0,
            view_distance: 10,
            simulation_distance: 10,
            motd: "Test Server".to_string(),
            keep_alive_timeout: Duration::from_secs(30),
            enabled_dimensions: crate::hakoniwa::DimensionSet::default(),
        }
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

        let config = test_connection_config();

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

        let config = test_connection_config();

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

    #[test]
    fn status_response_from_config_uses_motd() {
        let response = status_response_from_config("My Custom Server", 50);
        assert_eq!(response.description.text, "My Custom Server");
        assert_eq!(response.players.max, 50);
        assert_eq!(response.version.name, "1.20.1");
        assert_eq!(response.version.protocol, 763);
    }

    #[test]
    fn status_response_from_config_empty_motd() {
        let response = status_response_from_config("", 1);
        assert_eq!(response.description.text, "");
        assert_eq!(response.players.max, 1);
    }

    #[test]
    fn default_status_response_has_defaults() {
        let response = default_status_response();
        assert_eq!(response.description.text, "A Rust Minecraft Server");
        assert_eq!(response.players.max, 20);
    }
}
