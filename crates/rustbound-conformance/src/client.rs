//! Black-box TCP client for the protocol 763 Status exchange.
//!
//! Connects to a configurable host and port, performs the Handshake -> Status
//! -> Ping exchange, and returns a semantic snapshot. The oracle path and Java
//! executable are supplied by the caller at runtime; this crate never
//! references or commits them.

use std::fmt;
use std::time::Duration;

use rustbound_protocol::framing::{PROTOCOL_MAX_FRAME_LENGTH, decode_frame, encode_frame};
use rustbound_protocol::handshake::{HandshakePacket, encode_handshake};
use rustbound_protocol::primitives::{decode_i64, decode_string, encode_i64};
use rustbound_protocol::state::NextState;
use rustbound_protocol::status::{MAX_STATUS_JSON_UTF16_UNITS, StatusResponse};

use crate::snapshot::StatusSnapshot;

/// An error encountered while running a status conformance probe.
#[derive(Debug)]
pub enum StatusClientError {
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

impl fmt::Display for StatusClientError {
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

impl std::error::Error for StatusClientError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Connect(error) | Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<std::io::Error> for StatusClientError {
    fn from(error: std::io::Error) -> Self {
        if error.kind() == std::io::ErrorKind::TimedOut
            || error.kind() == std::io::ErrorKind::WouldBlock
        {
            Self::Timeout
        } else {
            Self::Io(error)
        }
    }
}

/// A black-box status conformance client.
pub struct StatusClient {
    host: String,
    port: u16,
    timeout: Duration,
}

impl StatusClient {
    /// Creates a new client targeting the given host and port.
    pub fn new(host: impl Into<String>, port: u16) -> Self {
        Self {
            host: host.into(),
            port,
            timeout: Duration::from_secs(10),
        }
    }

    /// Sets the per-operation timeout.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Performs the full Status exchange and returns a semantic snapshot.
    ///
    /// The exchange consists of:
    /// 1. Send Handshake with `next_state = Status`.
    /// 2. Send Status Request (empty payload).
    /// 3. Receive Status Response (JSON).
    /// 4. Send Ping Request with an arbitrary `i64`.
    /// 5. Receive Pong Response and verify the echo.
    pub async fn probe(&self) -> Result<StatusSnapshot, StatusClientError> {
        self.probe_with_protocol(763).await
    }

    /// Like [`probe`](Self::probe) but allows specifying a custom protocol
    /// version for the handshake.
    pub async fn probe_with_protocol(
        &self,
        protocol_version: i32,
    ) -> Result<StatusSnapshot, StatusClientError> {
        let mut stream = tokio::time::timeout(
            self.timeout,
            tokio::net::TcpStream::connect((self.host.as_str(), self.port)),
        )
        .await
        .map_err(|_| StatusClientError::Timeout)?
        .map_err(StatusClientError::Connect)?;

        let mut buf = Vec::new();

        // 1. Send Handshake.
        let handshake = HandshakePacket {
            protocol_version,
            server_address: self.host.clone(),
            port: self.port,
            next_state: NextState::Status,
        };
        encode_handshake(&handshake, PROTOCOL_MAX_FRAME_LENGTH, &mut buf)
            .map_err(|error| StatusClientError::Protocol(error.to_string()))?;
        write_all(&mut stream, &buf, self.timeout).await?;
        buf.clear();

        // 2. Send Status Request (empty payload).
        encode_frame(0x00, &[], PROTOCOL_MAX_FRAME_LENGTH, &mut buf)
            .map_err(|error| StatusClientError::Protocol(error.to_string()))?;
        write_all(&mut stream, &buf, self.timeout).await?;
        buf.clear();

        // 3. Receive Status Response.
        let response = read_status_response(&mut stream, &mut buf, self.timeout).await?;

        // 4. Send Ping Request.
        let ping_payload: i64 = 0x1234_5678_9abc_def0i64;
        let mut ping_body = Vec::new();
        encode_i64(ping_payload, &mut ping_body);
        encode_frame(0x01, &ping_body, PROTOCOL_MAX_FRAME_LENGTH, &mut buf)
            .map_err(|error| StatusClientError::Protocol(error.to_string()))?;
        write_all(&mut stream, &buf, self.timeout).await?;
        buf.clear();

        // 5. Receive Pong Response and verify echo.
        let pong_echoed =
            read_pong_and_verify(&mut stream, &mut buf, ping_payload, self.timeout).await?;

        Ok(StatusSnapshot::from_response(&response, pong_echoed))
    }
}

async fn write_all(
    stream: &mut tokio::net::TcpStream,
    data: &[u8],
    timeout: Duration,
) -> Result<(), StatusClientError> {
    use tokio::io::AsyncWriteExt;
    tokio::time::timeout(timeout, stream.write_all(data))
        .await
        .map_err(|_| StatusClientError::Timeout)?
        .map_err(StatusClientError::from)?;
    Ok(())
}

/// Attempts to decode one frame from `buf`. Returns `Ok(Some(consumed))` when
/// a complete frame is available, with `consumed` being the number of bytes
/// the frame occupies. Returns `Ok(None)` when more data is needed.
fn try_decode_frame_len(buf: &[u8]) -> Result<Option<usize>, String> {
    let mut input: &[u8] = buf;
    match decode_frame(&mut input, PROTOCOL_MAX_FRAME_LENGTH) {
        Ok(rustbound_protocol::framing::DecodeOutcome::Complete(_)) => {
            Ok(Some(buf.len() - input.len()))
        }
        Ok(rustbound_protocol::framing::DecodeOutcome::Incomplete) => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

async fn read_status_response(
    stream: &mut tokio::net::TcpStream,
    buf: &mut Vec<u8>,
    timeout: Duration,
) -> Result<StatusResponse, StatusClientError> {
    use tokio::io::AsyncReadExt;

    loop {
        match try_decode_frame_len(buf) {
            Ok(Some(consumed)) => {
                // Extract the frame body without borrow issues.
                let frame_data: Vec<u8> = buf[..consumed].to_vec();
                buf.drain(..consumed);

                let mut input: &[u8] = &frame_data;
                let frame = decode_frame(&mut input, PROTOCOL_MAX_FRAME_LENGTH)
                    .map_err(|error| StatusClientError::Protocol(error.to_string()))?;
                let frame = match frame {
                    rustbound_protocol::framing::DecodeOutcome::Complete(f) => f,
                    rustbound_protocol::framing::DecodeOutcome::Incomplete => {
                        return Err(StatusClientError::Protocol(
                            "frame became incomplete after copy".to_owned(),
                        ));
                    }
                };

                if frame.packet_id != 0x00 {
                    return Err(StatusClientError::Protocol(format!(
                        "expected status response packet ID 0, got {}",
                        frame.packet_id
                    )));
                }
                let mut body = frame.payload;
                let json = decode_string(&mut body, MAX_STATUS_JSON_UTF16_UNITS)
                    .map_err(|error| StatusClientError::Protocol(error.to_string()))?;
                let response: StatusResponse = serde_json::from_str(json).map_err(|error| {
                    StatusClientError::Protocol(format!("JSON parse error: {error}"))
                })?;
                return Ok(response);
            }
            Ok(None) => {
                let mut tmp = [0u8; 4096];
                let n = tokio::time::timeout(timeout, stream.read(&mut tmp))
                    .await
                    .map_err(|_| StatusClientError::Timeout)?
                    .map_err(StatusClientError::from)?;
                if n == 0 {
                    return Err(StatusClientError::PrematureEof);
                }
                buf.extend_from_slice(&tmp[..n]);
            }
            Err(message) => {
                return Err(StatusClientError::Protocol(message));
            }
        }
    }
}

async fn read_pong_and_verify(
    stream: &mut tokio::net::TcpStream,
    buf: &mut Vec<u8>,
    expected: i64,
    timeout: Duration,
) -> Result<bool, StatusClientError> {
    use tokio::io::AsyncReadExt;

    loop {
        match try_decode_frame_len(buf) {
            Ok(Some(consumed)) => {
                let frame_data: Vec<u8> = buf[..consumed].to_vec();
                buf.drain(..consumed);

                let mut input: &[u8] = &frame_data;
                let frame = decode_frame(&mut input, PROTOCOL_MAX_FRAME_LENGTH)
                    .map_err(|error| StatusClientError::Protocol(error.to_string()))?;
                let frame = match frame {
                    rustbound_protocol::framing::DecodeOutcome::Complete(f) => f,
                    rustbound_protocol::framing::DecodeOutcome::Incomplete => {
                        return Err(StatusClientError::Protocol(
                            "frame became incomplete after copy".to_owned(),
                        ));
                    }
                };

                if frame.packet_id != 0x01 {
                    return Err(StatusClientError::Protocol(format!(
                        "expected pong packet ID 1, got {}",
                        frame.packet_id
                    )));
                }
                let mut body = frame.payload;
                let value = decode_i64(&mut body)
                    .map_err(|error| StatusClientError::Protocol(error.to_string()))?;
                return Ok(value == expected);
            }
            Ok(None) => {
                let mut tmp = [0u8; 4096];
                let n = tokio::time::timeout(timeout, stream.read(&mut tmp))
                    .await
                    .map_err(|_| StatusClientError::Timeout)?
                    .map_err(StatusClientError::from)?;
                if n == 0 {
                    return Err(StatusClientError::PrematureEof);
                }
                buf.extend_from_slice(&tmp[..n]);
            }
            Err(message) => {
                return Err(StatusClientError::Protocol(message));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{StatusClient, StatusClientError};
    use rustbound_protocol::framing::{PROTOCOL_MAX_FRAME_LENGTH, decode_frame, encode_frame};
    use rustbound_protocol::primitives::{decode_i64, encode_i64, encode_string};
    use rustbound_protocol::status::{
        StatusDescription, StatusPlayers, StatusResponse, StatusVersion,
    };
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    enum MockBehavior {
        Normal,
        PrematureEof,
        WrongPong,
        MalformedJson,
    }

    /// A mock status server that handles a single client connection.
    async fn run_mock_status_server(
        listener: TcpListener,
        response: StatusResponse,
        behavior: MockBehavior,
    ) {
        let (mut socket, _) = listener
            .accept()
            .await
            .unwrap_or_else(|e| panic!("accept failed: {e}"));

        let mut buf = vec![0u8; 4096];
        let mut accumulated = Vec::new();

        // Read handshake + status request.
        loop {
            let n = socket
                .read(&mut buf)
                .await
                .unwrap_or_else(|e| panic!("read failed: {e}"));
            if n == 0 {
                return;
            }
            accumulated.extend_from_slice(&buf[..n]);

            // Try to consume handshake frame.
            let mut input: &[u8] = &accumulated;
            if let Ok(rustbound_protocol::framing::DecodeOutcome::Complete(frame)) =
                decode_frame(&mut input, PROTOCOL_MAX_FRAME_LENGTH)
            {
                if frame.packet_id == 0x00 {
                    let consumed = accumulated.len() - input.len();
                    accumulated.drain(..consumed);

                    // Try to consume status request.
                    let mut input2: &[u8] = &accumulated;
                    if let Ok(rustbound_protocol::framing::DecodeOutcome::Complete(frame2)) =
                        decode_frame(&mut input2, PROTOCOL_MAX_FRAME_LENGTH)
                    {
                        if frame2.packet_id == 0x00 {
                            let consumed2 = accumulated.len() - input2.len();
                            accumulated.drain(..consumed2);
                            break;
                        }
                    }
                    // Need more data for status request; continue reading.
                }
            }
        }

        match behavior {
            MockBehavior::Normal => {
                // Send status response.
                let json = serde_json::to_string(&response).unwrap_or_else(|e| panic!("json: {e}"));
                let mut body = Vec::new();
                encode_string(&json, 32767, &mut body).unwrap_or_else(|e| panic!("encode: {e}"));
                let mut wire = Vec::new();
                encode_frame(0x00, &body, PROTOCOL_MAX_FRAME_LENGTH, &mut wire)
                    .unwrap_or_else(|e| panic!("frame: {e}"));
                socket
                    .write_all(&wire)
                    .await
                    .unwrap_or_else(|e| panic!("write: {e}"));

                // Read ping and send pong.
                loop {
                    if !accumulated.is_empty() {
                        let mut input: &[u8] = &accumulated;
                        if let Ok(rustbound_protocol::framing::DecodeOutcome::Complete(frame)) =
                            decode_frame(&mut input, PROTOCOL_MAX_FRAME_LENGTH)
                        {
                            if frame.packet_id == 0x01 {
                                let mut payload = frame.payload;
                                let value = decode_i64(&mut payload)
                                    .unwrap_or_else(|e| panic!("decode i64: {e}"));
                                let mut pong_body = Vec::new();
                                encode_i64(value, &mut pong_body);
                                let mut pong_wire = Vec::new();
                                encode_frame(
                                    0x01,
                                    &pong_body,
                                    PROTOCOL_MAX_FRAME_LENGTH,
                                    &mut pong_wire,
                                )
                                .unwrap_or_else(|e| panic!("frame: {e}"));
                                socket
                                    .write_all(&pong_wire)
                                    .await
                                    .unwrap_or_else(|e| panic!("write: {e}"));
                                return;
                            }
                            let consumed = accumulated.len() - input.len();
                            accumulated.drain(..consumed);
                            continue;
                        }
                        accumulated.clear();
                    }
                    let n = socket
                        .read(&mut buf)
                        .await
                        .unwrap_or_else(|e| panic!("read: {e}"));
                    if n == 0 {
                        return;
                    }
                    accumulated.extend_from_slice(&buf[..n]);
                }
            }
            MockBehavior::PrematureEof => {
                // Close connection after receiving status request.
            }
            MockBehavior::WrongPong => {
                // Send status response.
                let json = serde_json::to_string(&response).unwrap_or_else(|e| panic!("json: {e}"));
                let mut body = Vec::new();
                encode_string(&json, 32767, &mut body).unwrap_or_else(|e| panic!("encode: {e}"));
                let mut wire = Vec::new();
                encode_frame(0x00, &body, PROTOCOL_MAX_FRAME_LENGTH, &mut wire)
                    .unwrap_or_else(|e| panic!("frame: {e}"));
                socket
                    .write_all(&wire)
                    .await
                    .unwrap_or_else(|e| panic!("write: {e}"));

                // Read ping and send wrong pong.
                loop {
                    if !accumulated.is_empty() {
                        let mut input: &[u8] = &accumulated;
                        if let Ok(rustbound_protocol::framing::DecodeOutcome::Complete(frame)) =
                            decode_frame(&mut input, PROTOCOL_MAX_FRAME_LENGTH)
                        {
                            if frame.packet_id == 0x01 {
                                let mut pong_body = Vec::new();
                                encode_i64(0, &mut pong_body);
                                let mut pong_wire = Vec::new();
                                encode_frame(
                                    0x01,
                                    &pong_body,
                                    PROTOCOL_MAX_FRAME_LENGTH,
                                    &mut pong_wire,
                                )
                                .unwrap_or_else(|e| panic!("frame: {e}"));
                                socket
                                    .write_all(&pong_wire)
                                    .await
                                    .unwrap_or_else(|e| panic!("write: {e}"));
                                return;
                            }
                            let consumed = accumulated.len() - input.len();
                            accumulated.drain(..consumed);
                            continue;
                        }
                        accumulated.clear();
                    }
                    let n = socket
                        .read(&mut buf)
                        .await
                        .unwrap_or_else(|e| panic!("read: {e}"));
                    if n == 0 {
                        return;
                    }
                    accumulated.extend_from_slice(&buf[..n]);
                }
            }
            MockBehavior::MalformedJson => {
                let bad_json = "{not valid json";
                let mut body = Vec::new();
                encode_string(bad_json, 32767, &mut body).unwrap_or_else(|e| panic!("encode: {e}"));
                let mut wire = Vec::new();
                encode_frame(0x00, &body, PROTOCOL_MAX_FRAME_LENGTH, &mut wire)
                    .unwrap_or_else(|e| panic!("frame: {e}"));
                socket
                    .write_all(&wire)
                    .await
                    .unwrap_or_else(|e| panic!("write: {e}"));
            }
        }
    }

    fn sample_response() -> StatusResponse {
        StatusResponse {
            version: StatusVersion {
                name: "1.20.1".to_owned(),
                protocol: 763,
            },
            players: StatusPlayers {
                max: 20,
                online: 0,
                sample: None,
            },
            description: StatusDescription {
                text: "Test Server".to_owned(),
            },
            favicon: None,
        }
    }

    #[tokio::test]
    async fn normal_exchange_succeeds() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .unwrap_or_else(|e| panic!("bind: {e}"));
        let addr = listener
            .local_addr()
            .unwrap_or_else(|e| panic!("addr: {e}"));
        let response = sample_response();

        let server = tokio::spawn(run_mock_status_server(
            listener,
            response,
            MockBehavior::Normal,
        ));
        let client =
            StatusClient::new("127.0.0.1", addr.port()).with_timeout(Duration::from_secs(5));
        let snapshot = client
            .probe()
            .await
            .unwrap_or_else(|e| panic!("probe: {e}"));

        assert_eq!(snapshot.version_name, "1.20.1");
        assert_eq!(snapshot.protocol_version, 763);
        assert_eq!(snapshot.max_players, 20);
        assert_eq!(snapshot.online_players, 0);
        assert_eq!(snapshot.description_text, "Test Server");
        assert!(snapshot.pong_echoed);

        let _ = server.await;
    }

    #[tokio::test]
    async fn premature_eof_is_detected() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .unwrap_or_else(|e| panic!("bind: {e}"));
        let addr = listener
            .local_addr()
            .unwrap_or_else(|e| panic!("addr: {e}"));
        let response = sample_response();

        let server = tokio::spawn(run_mock_status_server(
            listener,
            response,
            MockBehavior::PrematureEof,
        ));
        let client =
            StatusClient::new("127.0.0.1", addr.port()).with_timeout(Duration::from_secs(5));
        let result = client.probe().await;

        assert!(matches!(result, Err(StatusClientError::PrematureEof)));
        let _ = server.await;
    }

    #[tokio::test]
    async fn wrong_pong_is_detected() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .unwrap_or_else(|e| panic!("bind: {e}"));
        let addr = listener
            .local_addr()
            .unwrap_or_else(|e| panic!("addr: {e}"));
        let response = sample_response();

        let server = tokio::spawn(run_mock_status_server(
            listener,
            response,
            MockBehavior::WrongPong,
        ));
        let client =
            StatusClient::new("127.0.0.1", addr.port()).with_timeout(Duration::from_secs(5));
        let snapshot = client
            .probe()
            .await
            .unwrap_or_else(|e| panic!("probe: {e}"));

        assert!(!snapshot.pong_echoed);
        let _ = server.await;
    }

    #[tokio::test]
    async fn malformed_json_is_detected() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .unwrap_or_else(|e| panic!("bind: {e}"));
        let addr = listener
            .local_addr()
            .unwrap_or_else(|e| panic!("addr: {e}"));
        let response = sample_response();

        let server = tokio::spawn(run_mock_status_server(
            listener,
            response,
            MockBehavior::MalformedJson,
        ));
        let client =
            StatusClient::new("127.0.0.1", addr.port()).with_timeout(Duration::from_secs(5));
        let result = client.probe().await;

        assert!(matches!(result, Err(StatusClientError::Protocol(_))));
        let _ = server.await;
    }

    #[tokio::test]
    async fn connection_refusal_is_detected() {
        // Bind a socket, get its port, then drop it. The port should not
        // be listening anymore, causing a connection refusal.
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .unwrap_or_else(|e| panic!("bind: {e}"));
        let addr = listener
            .local_addr()
            .unwrap_or_else(|e| panic!("addr: {e}"));
        drop(listener);

        // Give the OS a moment to release the socket.
        tokio::time::sleep(Duration::from_millis(50)).await;

        let client =
            StatusClient::new("127.0.0.1", addr.port()).with_timeout(Duration::from_secs(2));
        let result = client.probe().await;
        assert!(
            matches!(
                result,
                Err(StatusClientError::Connect(_))
                    | Err(StatusClientError::Io(_))
                    | Err(StatusClientError::Timeout)
            ),
            "expected connection error, got {result:?}"
        );
    }

    #[tokio::test]
    async fn timeout_is_detected() {
        // Bind a socket but never accept connections.
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .unwrap_or_else(|e| panic!("bind: {e}"));
        let addr = listener
            .local_addr()
            .unwrap_or_else(|e| panic!("addr: {e}"));

        let client =
            StatusClient::new("127.0.0.1", addr.port()).with_timeout(Duration::from_millis(100));
        let result = client.probe().await;
        assert!(matches!(
            result,
            Err(StatusClientError::Timeout
                | StatusClientError::PrematureEof
                | StatusClientError::Io(_))
        ));
        drop(listener);
    }
}
