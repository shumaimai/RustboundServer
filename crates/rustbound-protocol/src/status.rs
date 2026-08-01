//! Protocol 763 Status state exchange: server-list ping and pong.
//!
//! After a handshake with `next_state = 1`, the client enters the Status state
//! and performs a four-step exchange:
//!
//! 1. Client sends Status Request (`0x00`, empty payload).
//! 2. Server responds with Status Response (`0x00`, JSON string).
//! 3. Client sends Ping Request (`0x01`, signed `i64`).
//! 4. Server responds with Pong Response (`0x01`, identical `i64`).
//!
//! After the pong the server closes the connection.

use std::fmt;

use crate::framing::{DecodeOutcome, PacketFrame, decode_frame, encode_frame};
use crate::primitives::{CodecError, decode_i64, decode_string, encode_i64, encode_string};
use crate::state::ProtocolState;

/// Packet ID for the serverbound and clientbound Status Request/Response.
pub const STATUS_PACKET_ID: i32 = 0x00;

/// Packet ID for the serverbound Ping and clientbound Pong.
pub const PING_PACKET_ID: i32 = 0x01;

/// Maximum UTF-16 unit count for the JSON status response string.
///
/// The vanilla server does not document an explicit bound, but the string is
/// carried as a protocol String so it inherits the standard three-bytes-per-
/// unit ceiling. 32767 units matches the vanilla chat component bound.
pub const MAX_STATUS_JSON_UTF16_UNITS: usize = 32767;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// An error encountered while encoding or decoding a Status state packet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatusError {
    /// A primitive codec failed.
    Codec(CodecError),
    /// The Status Request payload was not empty.
    PayloadNotEmpty { count: usize },
    /// The packet ID does not match the expected Status state packet.
    WrongPacketId { received: i32, expected: i32 },
    /// The JSON response string exceeded the configured UTF-16 unit bound.
    JsonTooLong,
    /// The JSON value could not be serialized or deserialized.
    JsonSerialization(String),
    /// More input is required.
    Incomplete,
}

impl fmt::Display for StatusError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Codec(error) => write!(formatter, "status codec error: {error}"),
            Self::PayloadNotEmpty { count } => {
                write!(
                    formatter,
                    "status payload must be empty, got {count} byte(s)"
                )
            }
            Self::WrongPacketId { received, expected } => {
                write!(
                    formatter,
                    "expected status packet ID {expected}, got {received}"
                )
            }
            Self::JsonTooLong => formatter.write_str("status JSON response exceeds limit"),
            Self::JsonSerialization(message) => {
                write!(formatter, "status JSON serialization failed: {message}")
            }
            Self::Incomplete => formatter.write_str("status input is incomplete"),
        }
    }
}

impl std::error::Error for StatusError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Codec(error) => Some(error),
            _ => None,
        }
    }
}

impl From<CodecError> for StatusError {
    fn from(error: CodecError) -> Self {
        match error {
            CodecError::IncompleteInput => Self::Incomplete,
            other => Self::Codec(other),
        }
    }
}

// ---------------------------------------------------------------------------
// Status Response data model
// ---------------------------------------------------------------------------

/// The version block of a Status Response.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct StatusVersion {
    /// Human-readable version name, e.g. `"1.20.1"`.
    pub name: String,
    /// Protocol version number, e.g. `763`.
    pub protocol: i32,
}

/// A single entry in the players sample list.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PlayerSampleEntry {
    /// The player's display name.
    pub name: String,
    /// The player's UUID as a hyphenated string.
    pub id: String,
}

/// The players block of a Status Response.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct StatusPlayers {
    /// Maximum number of players the server accepts.
    pub max: i32,
    /// Current online player count.
    pub online: i32,
    /// Optional sample of online players. Omitted from JSON when `None`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sample: Option<Vec<PlayerSampleEntry>>,
}

/// The description block, modeled as a simple text chat component.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct StatusDescription {
    /// The plain-text description.
    pub text: String,
}

/// The complete Status Response payload.
///
/// `favicon` and `players.sample` are optional and omitted from JSON when
/// `None`, matching the vanilla server's behavior.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct StatusResponse {
    /// Server version information.
    pub version: StatusVersion,
    /// Player count and optional sample.
    pub players: StatusPlayers,
    /// Plain-text server description.
    pub description: StatusDescription,
    /// Optional base64-encoded favicon data URI. Omitted when `None`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub favicon: Option<String>,
}

// ---------------------------------------------------------------------------
// Status Request (serverbound 0x00)
// ---------------------------------------------------------------------------

/// Decodes a serverbound Status Request (packet ID `0x00`).
///
/// The payload must be empty. Returns `Ok(None)` when the input is incomplete
/// (input is left unchanged in that case). The caller is responsible for state
/// enforcement.
pub fn decode_status_request(
    input: &mut &[u8],
    max_frame_length: usize,
) -> Result<Option<()>, StatusError> {
    let source = *input;
    let outcome = decode_frame(input, max_frame_length).map_err(|_| StatusError::Incomplete)?;
    let frame = match outcome {
        DecodeOutcome::Complete(frame) => frame,
        DecodeOutcome::Incomplete => {
            *input = source;
            return Ok(None);
        }
    };

    validate_status_request_frame(&frame).inspect_err(|_| {
        *input = source;
    })?;
    Ok(Some(()))
}

/// Validates that a frame is a well-formed Status Request.
fn validate_status_request_frame(frame: &PacketFrame<'_>) -> Result<(), StatusError> {
    if frame.packet_id != STATUS_PACKET_ID {
        return Err(StatusError::WrongPacketId {
            received: frame.packet_id,
            expected: STATUS_PACKET_ID,
        });
    }
    if !frame.payload.is_empty() {
        return Err(StatusError::PayloadNotEmpty {
            count: frame.payload.len(),
        });
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Status Response (clientbound 0x00)
// ---------------------------------------------------------------------------

/// Encodes a clientbound Status Response (packet ID `0x00`) with a JSON body.
///
/// The response is serialized to JSON, encoded as a protocol String, and
/// wrapped in a frame. On error, `output` is unchanged.
pub fn encode_status_response(
    response: &StatusResponse,
    max_frame_length: usize,
    output: &mut Vec<u8>,
) -> Result<(), StatusError> {
    let json = serde_json::to_string(response)
        .map_err(|error| StatusError::JsonSerialization(error.to_string()))?;

    let mut body = Vec::new();
    encode_string(&json, MAX_STATUS_JSON_UTF16_UNITS, &mut body).map_err(|error| match error {
        CodecError::StringTooLong => StatusError::JsonTooLong,
        other => StatusError::from(other),
    })?;

    encode_frame(STATUS_PACKET_ID, &body, max_frame_length, output)
        .map_err(|_| StatusError::Incomplete)
}

/// Decodes a clientbound Status Response (packet ID `0x00`) from a frame.
///
/// Returns `Ok(None)` when the input is incomplete. The caller is responsible
/// for state enforcement.
pub fn decode_status_response(
    input: &mut &[u8],
    max_frame_length: usize,
) -> Result<Option<StatusResponse>, StatusError> {
    let source = *input;
    let outcome = decode_frame(input, max_frame_length).map_err(|_| StatusError::Incomplete)?;
    let frame = match outcome {
        DecodeOutcome::Complete(frame) => frame,
        DecodeOutcome::Incomplete => {
            *input = source;
            return Ok(None);
        }
    };

    let response = parse_status_response_body(&frame).inspect_err(|_| {
        *input = source;
    })?;
    Ok(Some(response))
}

fn parse_status_response_body(frame: &PacketFrame<'_>) -> Result<StatusResponse, StatusError> {
    if frame.packet_id != STATUS_PACKET_ID {
        return Err(StatusError::WrongPacketId {
            received: frame.packet_id,
            expected: STATUS_PACKET_ID,
        });
    }

    let mut body = frame.payload;
    let json =
        decode_string(&mut body, MAX_STATUS_JSON_UTF16_UNITS).map_err(|error| match error {
            CodecError::StringTooLong => StatusError::JsonTooLong,
            other => StatusError::from(other),
        })?;

    serde_json::from_str::<StatusResponse>(json)
        .map_err(|error| StatusError::JsonSerialization(error.to_string()))
}

// ---------------------------------------------------------------------------
// Ping Request / Pong Response (serverbound/clientbound 0x01)
// ---------------------------------------------------------------------------

/// Decodes a serverbound Ping Request (packet ID `0x01`).
///
/// Returns `Ok(None)` when the input is incomplete. The caller is responsible
/// for state enforcement.
pub fn decode_ping_request(
    input: &mut &[u8],
    max_frame_length: usize,
) -> Result<Option<i64>, StatusError> {
    let source = *input;
    let outcome = decode_frame(input, max_frame_length).map_err(|_| StatusError::Incomplete)?;
    let frame = match outcome {
        DecodeOutcome::Complete(frame) => frame,
        DecodeOutcome::Incomplete => {
            *input = source;
            return Ok(None);
        }
    };

    let payload = parse_ping_body(&frame).inspect_err(|_| {
        *input = source;
    })?;
    Ok(Some(payload))
}

fn parse_ping_body(frame: &PacketFrame<'_>) -> Result<i64, StatusError> {
    if frame.packet_id != PING_PACKET_ID {
        return Err(StatusError::WrongPacketId {
            received: frame.packet_id,
            expected: PING_PACKET_ID,
        });
    }

    let mut body = frame.payload;
    let value = decode_i64(&mut body).map_err(StatusError::from)?;
    if !body.is_empty() {
        return Err(StatusError::PayloadNotEmpty { count: body.len() });
    }
    Ok(value)
}

/// Encodes a clientbound Pong Response (packet ID `0x01`) echoing the ping
/// payload byte-for-byte.
///
/// On error, `output` is unchanged.
pub fn encode_pong_response(
    payload: i64,
    max_frame_length: usize,
    output: &mut Vec<u8>,
) -> Result<(), StatusError> {
    let mut body = Vec::new();
    encode_i64(payload, &mut body);

    encode_frame(PING_PACKET_ID, &body, max_frame_length, output)
        .map_err(|_| StatusError::Incomplete)
}

// ---------------------------------------------------------------------------
// State routing helpers
// ---------------------------------------------------------------------------

/// Returns `Ok(())` if the given state is valid for receiving a Status packet.
pub fn ensure_status_state(current: ProtocolState) -> Result<(), StatusStateError> {
    match current {
        ProtocolState::Status => Ok(()),
        ProtocolState::Closed => Err(StatusStateError::ConnectionClosed),
        other => Err(StatusStateError::WrongState {
            current: other,
            expected: ProtocolState::Status,
        }),
    }
}

/// An error produced while enforcing Status state routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusStateError {
    /// A packet was received in a state that does not accept it.
    WrongState {
        current: ProtocolState,
        expected: ProtocolState,
    },
    /// The connection is closed.
    ConnectionClosed,
}

impl fmt::Display for StatusStateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongState { current, expected } => {
                write!(
                    formatter,
                    "status packet received in {current} state, expected {expected}"
                )
            }
            Self::ConnectionClosed => formatter.write_str("connection is closed"),
        }
    }
}

impl std::error::Error for StatusStateError {}

#[cfg(test)]
mod tests {
    use super::{
        PING_PACKET_ID, STATUS_PACKET_ID, StatusDescription, StatusError, StatusPlayers,
        StatusResponse, StatusStateError, StatusVersion, decode_ping_request,
        decode_status_request, decode_status_response, encode_pong_response,
        encode_status_response, ensure_status_state,
    };
    use crate::framing::encode_frame;
    use crate::primitives::{encode_i64, encode_string};
    use crate::state::ProtocolState;

    const TEST_MAX_FRAME: usize = 4096;

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
                text: "A Minecraft Server".to_owned(),
            },
            favicon: None,
        }
    }

    // --- Status Request ---

    #[test]
    fn empty_status_request_decodes_successfully() -> Result<(), StatusError> {
        let mut wire = Vec::new();
        encode_frame(STATUS_PACKET_ID, &[], TEST_MAX_FRAME, &mut wire)
            .map_err(|_| StatusError::Incomplete)?;

        let mut input = wire.as_slice();
        assert_eq!(
            decode_status_request(&mut input, TEST_MAX_FRAME),
            Ok(Some(()))
        );
        assert!(input.is_empty());
        Ok(())
    }

    #[test]
    fn non_empty_status_request_is_rejected() -> Result<(), StatusError> {
        let mut wire = Vec::new();
        encode_frame(STATUS_PACKET_ID, &[0xff, 0x00], TEST_MAX_FRAME, &mut wire)
            .map_err(|_| StatusError::Incomplete)?;

        let mut input = wire.as_slice();
        assert_eq!(
            decode_status_request(&mut input, TEST_MAX_FRAME),
            Err(StatusError::PayloadNotEmpty { count: 2 })
        );
        assert_eq!(input, wire.as_slice());
        Ok(())
    }

    #[test]
    fn status_request_with_wrong_packet_id_is_rejected() -> Result<(), StatusError> {
        let mut wire = Vec::new();
        encode_frame(0x02, &[], TEST_MAX_FRAME, &mut wire).map_err(|_| StatusError::Incomplete)?;

        let mut input = wire.as_slice();
        assert_eq!(
            decode_status_request(&mut input, TEST_MAX_FRAME),
            Err(StatusError::WrongPacketId {
                received: 0x02,
                expected: STATUS_PACKET_ID,
            })
        );
        assert_eq!(input, wire.as_slice());
        Ok(())
    }

    #[test]
    fn truncated_status_request_is_incomplete() -> Result<(), StatusError> {
        let mut wire = Vec::new();
        encode_frame(STATUS_PACKET_ID, &[], TEST_MAX_FRAME, &mut wire)
            .map_err(|_| StatusError::Incomplete)?;

        for split in 0..wire.len() {
            let mut input = &wire[..split];
            assert_eq!(decode_status_request(&mut input, TEST_MAX_FRAME), Ok(None));
            assert_eq!(input, &wire[..split]);
        }
        Ok(())
    }

    // --- Status Response ---

    #[test]
    fn status_response_round_trips_without_optionals() -> Result<(), StatusError> {
        let original = sample_response();
        let mut wire = Vec::new();
        encode_status_response(&original, TEST_MAX_FRAME, &mut wire)?;

        let mut input = wire.as_slice();
        assert_eq!(
            decode_status_response(&mut input, TEST_MAX_FRAME),
            Ok(Some(original))
        );
        assert!(input.is_empty());
        Ok(())
    }

    #[test]
    fn status_response_round_trips_with_favicon_and_sample() -> Result<(), StatusError> {
        let original = StatusResponse {
            version: StatusVersion {
                name: "1.20.1".to_owned(),
                protocol: 763,
            },
            players: StatusPlayers {
                max: 100,
                online: 2,
                sample: Some(vec![
                    super::PlayerSampleEntry {
                        name: "Player1".to_owned(),
                        id: "00000000-0000-0000-0000-000000000001".to_owned(),
                    },
                    super::PlayerSampleEntry {
                        name: "Player2".to_owned(),
                        id: "00000000-0000-0000-0000-000000000002".to_owned(),
                    },
                ]),
            },
            description: StatusDescription {
                text: "Welcome".to_owned(),
            },
            favicon: Some("data:image/png;base64,iVBORw0KGgo=".to_owned()),
        };

        let mut wire = Vec::new();
        encode_status_response(&original, TEST_MAX_FRAME, &mut wire)?;

        let mut input = wire.as_slice();
        assert_eq!(
            decode_status_response(&mut input, TEST_MAX_FRAME),
            Ok(Some(original))
        );
        Ok(())
    }

    #[test]
    fn status_response_json_shape_is_correct() -> Result<(), StatusError> {
        let response = sample_response();
        let mut wire = Vec::new();
        encode_status_response(&response, TEST_MAX_FRAME, &mut wire)?;

        let mut input = wire.as_slice();
        let decoded =
            decode_status_response(&mut input, TEST_MAX_FRAME)?.ok_or(StatusError::Incomplete)?;

        // Compare as JSON values to avoid key-order sensitivity.
        let original_json = serde_json::to_value(&response)
            .map_err(|error| StatusError::JsonSerialization(error.to_string()))?;
        let decoded_json = serde_json::to_value(&decoded)
            .map_err(|error| StatusError::JsonSerialization(error.to_string()))?;
        assert_eq!(original_json, decoded_json);

        // Verify required fields are present and optionals are absent.
        assert!(original_json.get("favicon").is_none());
        assert!(original_json["players"].get("sample").is_none());
        assert_eq!(original_json["version"]["name"], "1.20.1");
        assert_eq!(original_json["version"]["protocol"], 763);
        assert_eq!(original_json["players"]["max"], 20);
        assert_eq!(original_json["players"]["online"], 0);
        assert_eq!(original_json["description"]["text"], "A Minecraft Server");
        Ok(())
    }

    #[test]
    fn status_response_with_wrong_packet_id_is_rejected() -> Result<(), StatusError> {
        let json = serde_json::to_string(&sample_response())
            .map_err(|error| StatusError::JsonSerialization(error.to_string()))?;
        let mut body = Vec::new();
        encode_string(&json, super::MAX_STATUS_JSON_UTF16_UNITS, &mut body)?;
        let mut wire = Vec::new();
        encode_frame(0x05, &body, TEST_MAX_FRAME, &mut wire)
            .map_err(|_| StatusError::Incomplete)?;

        let mut input = wire.as_slice();
        assert_eq!(
            decode_status_response(&mut input, TEST_MAX_FRAME),
            Err(StatusError::WrongPacketId {
                received: 0x05,
                expected: STATUS_PACKET_ID,
            })
        );
        assert_eq!(input, wire.as_slice());
        Ok(())
    }

    #[test]
    fn status_response_is_deterministic_for_fixed_state() -> Result<(), StatusError> {
        let response = sample_response();
        let mut wire1 = Vec::new();
        encode_status_response(&response, TEST_MAX_FRAME, &mut wire1)?;
        let mut wire2 = Vec::new();
        encode_status_response(&response, TEST_MAX_FRAME, &mut wire2)?;
        assert_eq!(wire1, wire2);
        Ok(())
    }

    // --- Ping / Pong ---

    #[test]
    fn ping_request_round_trips_and_pong_echoes_byte_for_byte() -> Result<(), StatusError> {
        let mut ping_body = Vec::new();
        encode_i64(0x1234_5678_9abc_def0, &mut ping_body);
        let mut ping_wire = Vec::new();
        encode_frame(PING_PACKET_ID, &ping_body, TEST_MAX_FRAME, &mut ping_wire)
            .map_err(|_| StatusError::Incomplete)?;

        let mut input = ping_wire.as_slice();
        let payload =
            decode_ping_request(&mut input, TEST_MAX_FRAME)?.ok_or(StatusError::Incomplete)?;
        assert_eq!(payload, 0x1234_5678_9abc_def0);
        assert!(input.is_empty());

        let mut pong_wire = Vec::new();
        encode_pong_response(payload, TEST_MAX_FRAME, &mut pong_wire)?;

        // Pong payload should be byte-for-byte identical to ping payload.
        assert_eq!(ping_wire, pong_wire);
        Ok(())
    }

    #[test]
    fn extreme_ping_values_round_trip() -> Result<(), StatusError> {
        for value in [i64::MIN, -1, 0, 1, i64::MAX] {
            let mut body = Vec::new();
            encode_i64(value, &mut body);
            let mut wire = Vec::new();
            encode_frame(PING_PACKET_ID, &body, TEST_MAX_FRAME, &mut wire)
                .map_err(|_| StatusError::Incomplete)?;

            let mut input = wire.as_slice();
            let decoded =
                decode_ping_request(&mut input, TEST_MAX_FRAME)?.ok_or(StatusError::Incomplete)?;
            assert_eq!(decoded, value);
            assert!(input.is_empty());
        }
        Ok(())
    }

    #[test]
    fn ping_request_with_wrong_packet_id_is_rejected() -> Result<(), StatusError> {
        let mut body = Vec::new();
        encode_i64(42, &mut body);
        let mut wire = Vec::new();
        encode_frame(0x00, &body, TEST_MAX_FRAME, &mut wire)
            .map_err(|_| StatusError::Incomplete)?;

        let mut input = wire.as_slice();
        assert_eq!(
            decode_ping_request(&mut input, TEST_MAX_FRAME),
            Err(StatusError::WrongPacketId {
                received: 0x00,
                expected: PING_PACKET_ID,
            })
        );
        assert_eq!(input, wire.as_slice());
        Ok(())
    }

    #[test]
    fn ping_request_with_trailing_bytes_is_rejected() -> Result<(), StatusError> {
        let mut body = Vec::new();
        encode_i64(42, &mut body);
        body.push(0xff);
        let mut wire = Vec::new();
        encode_frame(PING_PACKET_ID, &body, TEST_MAX_FRAME, &mut wire)
            .map_err(|_| StatusError::Incomplete)?;

        let mut input = wire.as_slice();
        assert_eq!(
            decode_ping_request(&mut input, TEST_MAX_FRAME),
            Err(StatusError::PayloadNotEmpty { count: 1 })
        );
        assert_eq!(input, wire.as_slice());
        Ok(())
    }

    #[test]
    fn truncated_ping_request_is_incomplete() -> Result<(), StatusError> {
        let mut body = Vec::new();
        encode_i64(42, &mut body);
        let mut wire = Vec::new();
        encode_frame(PING_PACKET_ID, &body, TEST_MAX_FRAME, &mut wire)
            .map_err(|_| StatusError::Incomplete)?;

        for split in 0..wire.len() {
            let mut input = &wire[..split];
            assert_eq!(decode_ping_request(&mut input, TEST_MAX_FRAME), Ok(None));
            assert_eq!(input, &wire[..split]);
        }
        Ok(())
    }

    // --- State routing ---

    #[test]
    fn status_state_check_accepts_status_state() {
        assert_eq!(ensure_status_state(ProtocolState::Status), Ok(()));
    }

    #[test]
    fn status_state_check_rejects_wrong_state() {
        for state in [
            ProtocolState::Handshaking,
            ProtocolState::Login,
            ProtocolState::Play,
        ] {
            assert_eq!(
                ensure_status_state(state),
                Err(StatusStateError::WrongState {
                    current: state,
                    expected: ProtocolState::Status,
                })
            );
        }
    }

    #[test]
    fn status_state_check_rejects_closed_connection() {
        assert_eq!(
            ensure_status_state(ProtocolState::Closed),
            Err(StatusStateError::ConnectionClosed)
        );
    }

    // --- Integration: full status exchange ---

    #[test]
    fn full_status_exchange_over_in_memory_buffer() -> Result<(), StatusError> {
        // 1. Client sends Status Request.
        let mut request_wire = Vec::new();
        encode_frame(STATUS_PACKET_ID, &[], TEST_MAX_FRAME, &mut request_wire)
            .map_err(|_| StatusError::Incomplete)?;
        let mut input = request_wire.as_slice();
        assert_eq!(
            decode_status_request(&mut input, TEST_MAX_FRAME),
            Ok(Some(()))
        );
        assert!(input.is_empty());

        // 2. Server sends Status Response.
        let response = sample_response();
        let mut response_wire = Vec::new();
        encode_status_response(&response, TEST_MAX_FRAME, &mut response_wire)?;
        let mut input = response_wire.as_slice();
        assert_eq!(
            decode_status_response(&mut input, TEST_MAX_FRAME),
            Ok(Some(response))
        );
        assert!(input.is_empty());

        // 3. Client sends Ping Request.
        let mut ping_body = Vec::new();
        encode_i64(0xdead_beef_cafe_babeu64 as i64, &mut ping_body);
        let mut ping_wire = Vec::new();
        encode_frame(PING_PACKET_ID, &ping_body, TEST_MAX_FRAME, &mut ping_wire)
            .map_err(|_| StatusError::Incomplete)?;
        let mut input = ping_wire.as_slice();
        let ping_payload =
            decode_ping_request(&mut input, TEST_MAX_FRAME)?.ok_or(StatusError::Incomplete)?;
        assert!(input.is_empty());

        // 4. Server sends Pong Response (echo).
        let mut pong_wire = Vec::new();
        encode_pong_response(ping_payload, TEST_MAX_FRAME, &mut pong_wire)?;
        assert_eq!(ping_wire, pong_wire);

        Ok(())
    }

    #[test]
    fn repeated_status_requests_are_accepted() -> Result<(), StatusError> {
        let mut wire = Vec::new();
        for _ in 0..3 {
            encode_frame(STATUS_PACKET_ID, &[], TEST_MAX_FRAME, &mut wire)
                .map_err(|_| StatusError::Incomplete)?;
        }

        let mut input = wire.as_slice();
        for _ in 0..3 {
            assert_eq!(
                decode_status_request(&mut input, TEST_MAX_FRAME),
                Ok(Some(()))
            );
        }
        assert!(input.is_empty());
        Ok(())
    }

    #[test]
    fn malformed_status_response_json_is_rejected() -> Result<(), StatusError> {
        let bad_json = "{not valid json";
        let mut body = Vec::new();
        encode_string(bad_json, super::MAX_STATUS_JSON_UTF16_UNITS, &mut body)?;
        let mut wire = Vec::new();
        encode_frame(STATUS_PACKET_ID, &body, TEST_MAX_FRAME, &mut wire)
            .map_err(|_| StatusError::Incomplete)?;

        let mut input = wire.as_slice();
        match decode_status_response(&mut input, TEST_MAX_FRAME) {
            Err(StatusError::JsonSerialization(_)) => {}
            other => panic!("expected JsonSerialization error, got {other:?}"),
        }
        assert_eq!(input, wire.as_slice());
        Ok(())
    }
}
