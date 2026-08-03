//! TCP listener and connection acceptor for the Rustbound server.
//!
//! Binds to a configurable host:port and accepts incoming connections.
//! Each accepted connection is handed off to a connection handler via
//! a channel. The listener runs on its own thread and supports graceful
//! shutdown.

use std::fmt;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// Configuration for the TCP listener.
#[derive(Debug, Clone)]
pub struct ListenerConfig {
    /// The host to bind to.
    pub host: String,
    /// The port to bind to.
    pub port: u16,
    /// Connection timeout for idle connections.
    pub connection_timeout: Duration,
    /// Whether to set TCP_NODELAY on accepted connections.
    pub tcp_nodelay: bool,
}

impl Default for ListenerConfig {
    fn default() -> Self {
        Self {
            host: "0.0.0.0".to_string(),
            port: 25565,
            connection_timeout: Duration::from_secs(30),
            tcp_nodelay: true,
        }
    }
}

/// An error encountered while starting or running the TCP listener.
#[derive(Debug)]
pub enum ListenerError {
    /// Failed to bind to the specified address.
    Bind(std::io::Error),
    /// Failed to set socket options.
    SocketOption(std::io::Error),
    /// The listener has been shut down.
    Shutdown,
}

impl fmt::Display for ListenerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bind(error) => write!(formatter, "failed to bind: {error}"),
            Self::SocketOption(error) => write!(formatter, "socket option error: {error}"),
            Self::Shutdown => formatter.write_str("listener has been shut down"),
        }
    }
}

impl std::error::Error for ListenerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Bind(error) | Self::SocketOption(error) => Some(error),
            _ => None,
        }
    }
}

/// A handle to a running TCP listener that supports graceful shutdown.
pub struct ListenerHandle {
    shutdown: Arc<AtomicBool>,
    bind_addr: SocketAddr,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl ListenerHandle {
    /// Returns the address the listener is bound to.
    pub fn bind_addr(&self) -> SocketAddr {
        self.bind_addr
    }

    /// Signals the listener to shut down and waits for the thread to exit.
    pub fn shutdown(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for ListenerHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Starts the TCP listener on a new thread.
///
/// The `handler` closure is called for each accepted connection, receiving
/// the `TcpStream` and its remote address. The handler runs on the acceptor
/// thread, so it should spawn its own thread for long-running connections.
///
/// The returned `ListenerHandle` can be used to shut down the listener.
pub fn start_listener<F>(
    config: ListenerConfig,
    handler: F,
) -> Result<ListenerHandle, ListenerError>
where
    F: Fn(TcpStream, SocketAddr) + Send + Sync + 'static,
{
    let addr: SocketAddr = format!("{}:{}", config.host, config.port).parse().map_err(
        |e: std::net::AddrParseError| {
            ListenerError::Bind(std::io::Error::new(std::io::ErrorKind::InvalidInput, e))
        },
    )?;

    let listener = TcpListener::bind(addr).map_err(ListenerError::Bind)?;
    listener
        .set_nonblocking(true)
        .map_err(ListenerError::SocketOption)?;

    let bind_addr = listener.local_addr().map_err(ListenerError::SocketOption)?;
    let shutdown = Arc::new(AtomicBool::new(false));
    let shutdown_clone = shutdown.clone();
    let connection_timeout = config.connection_timeout;
    let tcp_nodelay = config.tcp_nodelay;
    let handler = Arc::new(handler);

    let thread = std::thread::Builder::new()
        .name("rustbound-listener".to_string())
        .spawn(move || {
            run_accept_loop(
                listener,
                shutdown_clone,
                handler,
                connection_timeout,
                tcp_nodelay,
            );
        })
        .map_err(|e| ListenerError::Bind(std::io::Error::other(e)))?;

    Ok(ListenerHandle {
        shutdown,
        bind_addr,
        thread: Some(thread),
    })
}

fn run_accept_loop<F>(
    listener: TcpListener,
    shutdown: Arc<AtomicBool>,
    handler: Arc<F>,
    connection_timeout: Duration,
    tcp_nodelay: bool,
) where
    F: Fn(TcpStream, SocketAddr) + Send + Sync + 'static,
{
    // Use a short poll timeout so we can check the shutdown flag.
    let poll_timeout = Duration::from_millis(100);

    while !shutdown.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((stream, addr)) => {
                setup_connection(&stream, connection_timeout, tcp_nodelay);
                handler(stream, addr);
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(poll_timeout);
            }
            Err(e) => {
                eprintln!("listener: accept error: {e}");
                std::thread::sleep(poll_timeout);
            }
        }
    }
}

fn setup_connection(stream: &TcpStream, connection_timeout: Duration, tcp_nodelay: bool) {
    // On Windows, sockets accepted from a non-blocking listener inherit
    // non-blocking mode and then return WSAEWOULDBLOCK (10035) on read.
    // Force blocking I/O before applying timeouts.
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(connection_timeout));
    let _ = stream.set_write_timeout(Some(connection_timeout));
    if tcp_nodelay {
        let _ = stream.set_nodelay(true);
    }
}

#[cfg(test)]
mod tests {
    use super::{ListenerConfig, start_listener};
    use std::net::TcpStream;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    #[test]
    fn listener_config_default_is_valid() {
        let config = ListenerConfig::default();
        assert_eq!(config.port, 25565);
        assert_eq!(config.host, "0.0.0.0");
        assert_eq!(config.connection_timeout, Duration::from_secs(30));
        assert!(config.tcp_nodelay);
    }

    #[test]
    fn listener_accepts_connections() -> Result<(), Box<dyn std::error::Error>> {
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = counter.clone();

        let config = ListenerConfig {
            host: "127.0.0.1".to_string(),
            port: 0, // let OS choose
            ..Default::default()
        };

        let mut handle = start_listener(config, move |_stream, _addr| {
            counter_clone.fetch_add(1, Ordering::SeqCst);
        })?;

        // Give the listener a moment to start
        std::thread::sleep(Duration::from_millis(50));

        // Connect to the listener
        let addr = handle.bind_addr();
        let _stream = TcpStream::connect_timeout(&addr, Duration::from_secs(5))?;

        // Wait for the handler to be called
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(counter.load(Ordering::SeqCst), 1);

        handle.shutdown();
        Ok(())
    }

    #[test]
    fn listener_shutdown_stops_accepting() -> Result<(), Box<dyn std::error::Error>> {
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = counter.clone();

        let config = ListenerConfig {
            host: "127.0.0.1".to_string(),
            port: 0,
            ..Default::default()
        };

        let mut handle = start_listener(config, move |_stream, _addr| {
            counter_clone.fetch_add(1, Ordering::SeqCst);
        })?;

        std::thread::sleep(Duration::from_millis(50));
        handle.shutdown();

        // Try to connect after shutdown - should fail or not increment counter
        let addr = handle.bind_addr();
        let _ = TcpStream::connect_timeout(&addr, Duration::from_millis(100));
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(counter.load(Ordering::SeqCst), 0);

        Ok(())
    }

    #[test]
    fn listener_bind_error_on_invalid_host() {
        let config = ListenerConfig {
            host: "invalid_host_that_does_not_exist".to_string(),
            port: 25565,
            ..Default::default()
        };
        let result = start_listener(config, |_, _| {});
        assert!(result.is_err());
    }
}
