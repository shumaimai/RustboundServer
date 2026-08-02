//! Server orchestration: ties together the TCP listener, tick loop,
//! and connection handler into a single runnable server.

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::time::Duration;

use crate::config::ServerConfig;
use crate::connection::{ConnectionConfig, handle_connection, status_response_from_config};
use crate::listener::{ListenerConfig, ListenerHandle, start_listener};
use crate::session::EntityIdAllocator;
use crate::tick::{TickHandle, TickStartError, start_tick_loop};

/// An error encountered while running the server.
#[derive(Debug)]
pub enum ServerError {
    /// Failed to start the TCP listener.
    Listener(crate::listener::ListenerError),
    /// Failed to start the tick loop.
    Tick(TickStartError),
}

impl fmt::Display for ServerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Listener(error) => write!(formatter, "listener error: {error}"),
            Self::Tick(error) => write!(formatter, "tick error: {error}"),
        }
    }
}

impl std::error::Error for ServerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Listener(error) => Some(error),
            Self::Tick(error) => Some(error),
        }
    }
}

impl From<crate::listener::ListenerError> for ServerError {
    fn from(error: crate::listener::ListenerError) -> Self {
        Self::Listener(error)
    }
}

impl From<TickStartError> for ServerError {
    fn from(error: TickStartError) -> Self {
        Self::Tick(error)
    }
}

/// A running Rustbound server.
pub struct Server {
    listener: ListenerHandle,
    tick_handle: TickHandle,
}

impl Server {
    /// Starts the server with the given configuration.
    pub fn start(config: ServerConfig) -> Result<Self, ServerError> {
        let player_count = Arc::new(AtomicUsize::new(0));
        let (tick_handle, _event_rx) = start_tick_loop(
            player_count.clone(),
            config.level_name.clone(),
            config.autosave_interval_secs,
            Vec::new(), // no mods registered yet
        )?;

        let tick_sender = tick_handle.sender();
        let max_players = config.max_players;
        let connection_config = Arc::new(ConnectionConfig {
            status_response: status_response_from_config(&config.motd, config.max_players),
            online_mode: config.online_mode,
            max_frame_length: 65536,
            compression_threshold: config.network_compression_threshold,
            tick_sender,
            entity_id_allocator: EntityIdAllocator::new(1),
            play_read_timeout: Duration::from_secs(30),
            player_count: player_count.clone(),
            max_players,
            default_gamemode: config.default_gamemode,
            view_distance: config.view_distance,
            simulation_distance: config.simulation_distance,
            motd: config.motd.clone(),
            keep_alive_timeout: Duration::from_secs(config.keep_alive_timeout_secs),
        });

        let listener_config = ListenerConfig {
            host: config.host.clone(),
            port: config.port,
            connection_timeout: Duration::from_secs(config.connection_timeout_secs),
            tcp_nodelay: true,
        };

        let conn_config = connection_config.clone();
        let listener = start_listener(listener_config, move |stream, addr| {
            let conn_config = conn_config.clone();
            std::thread::spawn(move || {
                if let Err(e) = handle_connection(stream, &conn_config) {
                    eprintln!("connection from {addr} ended with error: {e}");
                }
            });
        })?;

        Ok(Self {
            listener,
            tick_handle,
        })
    }

    /// Returns the address the server is listening on.
    pub fn bind_addr(&self) -> std::net::SocketAddr {
        self.listener.bind_addr()
    }

    /// Shuts down the server gracefully.
    pub fn shutdown(&mut self) {
        self.listener.shutdown();
        self.tick_handle.shutdown();
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::Server;
    use crate::config::ServerConfig;
    use rustbound_conformance::{LoginProbeConfig, run_login_probe};
    use rustbound_protocol::framing::encode_frame;
    use rustbound_protocol::handshake::HandshakePacket;
    use rustbound_protocol::primitives::Uuid;
    use rustbound_protocol::state::NextState;
    use rustbound_protocol::status::decode_status_response;
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::time::Duration;

    #[test]
    fn server_starts_and_stops() -> Result<(), Box<dyn std::error::Error>> {
        let config = ServerConfig {
            host: "127.0.0.1".to_string(),
            port: 0,
            ..Default::default()
        };
        let mut server = Server::start(config)?;
        let addr = server.bind_addr();
        assert!(addr.port() > 0);
        server.shutdown();
        Ok(())
    }

    #[test]
    fn server_accepts_tcp_connections() -> Result<(), Box<dyn std::error::Error>> {
        let config = ServerConfig {
            host: "127.0.0.1".to_string(),
            port: 0,
            ..Default::default()
        };
        let mut server = Server::start(config)?;
        let addr = server.bind_addr();

        let _stream = TcpStream::connect_timeout(&addr, Duration::from_secs(5))?;
        std::thread::sleep(Duration::from_millis(50));

        server.shutdown();
        Ok(())
    }

    #[test]
    fn server_handles_status_exchange() -> Result<(), Box<dyn std::error::Error>> {
        let config = ServerConfig {
            host: "127.0.0.1".to_string(),
            port: 0,
            ..Default::default()
        };
        let mut server = Server::start(config)?;
        let addr = server.bind_addr();

        let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(5))?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        stream.set_write_timeout(Some(Duration::from_secs(5)))?;

        // Send handshake (next_state = Status)
        let handshake = HandshakePacket {
            protocol_version: 763,
            server_address: "127.0.0.1".to_string(),
            port: addr.port(),
            next_state: NextState::Status,
        };
        let mut hs_wire = Vec::new();
        rustbound_protocol::handshake::encode_handshake(&handshake, 65536, &mut hs_wire)?;
        stream.write_all(&hs_wire)?;

        // Send status request (empty frame with packet ID 0x00)
        let mut req_wire = Vec::new();
        encode_frame(0x00, &[], 65536, &mut req_wire)?;
        stream.write_all(&req_wire)?;

        // Read status response
        let mut buf = vec![0u8; 4096];
        let n = stream.read(&mut buf)?;
        let mut input = &buf[..n];
        let response = match decode_status_response(&mut input, 65536)? {
            Some(r) => r,
            None => panic!("expected status response"),
        };
        assert_eq!(response.version.protocol, 763);
        assert_eq!(response.version.name, "1.20.1");

        server.shutdown();
        Ok(())
    }

    #[test]
    fn server_handles_login_conformance() -> Result<(), Box<dyn std::error::Error>> {
        let config = ServerConfig {
            host: "127.0.0.1".to_string(),
            port: 0,
            online_mode: false,
            network_compression_threshold: -1,
            ..Default::default()
        };
        let mut server = Server::start(config)?;
        let addr = server.bind_addr();

        let probe_config = LoginProbeConfig {
            host: addr.ip().to_string(),
            port: addr.port(),
            username: "TestPlayer".to_string(),
            uuid: Uuid::new(0, 0),
            connect_timeout: Duration::from_secs(5),
            read_timeout: Duration::from_secs(5),
        };
        let outcome = run_login_probe(&probe_config)?;
        match outcome {
            rustbound_conformance::LoginOutcome::Success { username, .. } => {
                assert_eq!(username, "TestPlayer");
            }
            rustbound_conformance::LoginOutcome::Disconnect { reason } => {
                panic!("server disconnected: {reason}");
            }
        }

        server.shutdown();
        Ok(())
    }

    #[test]
    fn server_handles_play_conformance() -> Result<(), Box<dyn std::error::Error>> {
        let config = ServerConfig {
            host: "127.0.0.1".to_string(),
            port: 0,
            online_mode: false,
            network_compression_threshold: -1,
            ..Default::default()
        };
        let mut server = Server::start(config)?;
        let addr = server.bind_addr();

        let probe_config = rustbound_conformance::PlayProbeConfig {
            host: addr.ip().to_string(),
            port: addr.port(),
            username: "PlayTestPlayer".to_string(),
            uuid: Uuid::new(0, 0),
            connect_timeout: Duration::from_secs(5),
            read_timeout: Duration::from_secs(2),
        };
        let snapshot = rustbound_conformance::run_play_probe(&probe_config)?;

        assert_eq!(snapshot.username, "PlayTestPlayer");
        assert_eq!(snapshot.uuid, Uuid::new(0, 0));
        assert_eq!(snapshot.gamemode, 0); // Survival
        assert_eq!(snapshot.dimension_name, "minecraft:overworld");
        assert!(!snapshot.is_hardcore);
        assert!(snapshot.is_flat);
        // The probe should at least reach Join Game; teleport confirmation
        // depends on timing and may or may not complete within the read timeout.
        assert!(matches!(
            snapshot.phase_reached,
            rustbound_conformance::PlayPhase::JoinGame
                | rustbound_conformance::PlayPhase::TeleportConfirmed
                | rustbound_conformance::PlayPhase::PostTeleport
        ));

        server.shutdown();
        Ok(())
    }

    #[test]
    fn server_handles_play_conformance_with_compression() -> Result<(), Box<dyn std::error::Error>>
    {
        let config = ServerConfig {
            host: "127.0.0.1".to_string(),
            port: 0,
            online_mode: false,
            network_compression_threshold: 256, // default
            ..Default::default()
        };
        let mut server = Server::start(config)?;
        let addr = server.bind_addr();

        let probe_config = rustbound_conformance::PlayProbeConfig {
            host: addr.ip().to_string(),
            port: addr.port(),
            username: "PlayTestPlayer".to_string(),
            uuid: Uuid::new(0, 0),
            connect_timeout: Duration::from_secs(5),
            read_timeout: Duration::from_secs(2),
        };
        let snapshot = rustbound_conformance::run_play_probe(&probe_config)?;

        assert_eq!(snapshot.username, "PlayTestPlayer");
        assert_eq!(snapshot.dimension_name, "minecraft:overworld");

        server.shutdown();
        Ok(())
    }

    /// Oracle differential test: compares Rustbound's play snapshot against
    /// a local Forge 47.4.10 server.
    ///
    /// This test is `#[ignore]` by default. To run it:
    /// 1. Start a local Forge 47.4.10 server on `RUSTBOUND_ORACLE_HOST:RUSTBOUND_ORACLE_PORT`
    /// 2. Run: `cargo test -p rustbound-server -- --ignored oracle_play_differential`
    ///
    /// The oracle server must be in offline mode with the same world settings
    /// (flat world, survival, overworld) for a meaningful comparison.
    #[test]
    #[ignore]
    fn oracle_play_differential() -> Result<(), Box<dyn std::error::Error>> {
        let oracle_host =
            std::env::var("RUSTBOUND_ORACLE_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
        let oracle_port: u16 = std::env::var("RUSTBOUND_ORACLE_PORT")
            .unwrap_or_else(|_| "25565".to_string())
            .parse()
            .unwrap_or(25565);

        // Probe the oracle
        let oracle_config = rustbound_conformance::PlayProbeConfig {
            host: oracle_host,
            port: oracle_port,
            username: "DiffTestPlayer".to_string(),
            uuid: Uuid::new(0, 0),
            connect_timeout: Duration::from_secs(5),
            read_timeout: Duration::from_secs(10),
        };
        let oracle_snapshot = rustbound_conformance::run_play_probe(&oracle_config)?;

        // Probe Rustbound
        let config = ServerConfig {
            host: "127.0.0.1".to_string(),
            port: 0,
            online_mode: false,
            network_compression_threshold: -1,
            ..Default::default()
        };
        let mut server = Server::start(config)?;
        let addr = server.bind_addr();

        let rustbound_config = rustbound_conformance::PlayProbeConfig {
            host: addr.ip().to_string(),
            port: addr.port(),
            username: "DiffTestPlayer".to_string(),
            uuid: Uuid::new(0, 0),
            connect_timeout: Duration::from_secs(5),
            read_timeout: Duration::from_secs(10),
        };
        let rustbound_snapshot = rustbound_conformance::run_play_probe(&rustbound_config)?;

        server.shutdown();

        // Compare snapshots (nondeterministic fields are excluded by diff_play)
        let diff = rustbound_conformance::diff_play(&oracle_snapshot, &rustbound_snapshot);
        assert!(
            diff.result.is_match(),
            "play differential mismatch:\n{diff}"
        );

        Ok(())
    }
}
