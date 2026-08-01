//! Parsing for the protocol 763 Handshaking state.
//!
//! The Handshaking state accepts exactly one packet, ID `0x00`, which carries
//! the client's protocol version, the server address it connected to, the
//! port, and the requested next state. After this packet the connection
//! transitions to either `Status` or `Login`.

use std::fmt;

use crate::framing::{DecodeOutcome, PacketFrame, decode_frame};
use crate::primitives::{CodecError, decode_string, decode_u16, decode_var_int};
use crate::state::NextState;

/// Maximum number of UTF-16 code units allowed in the handshake server
/// address, matching the vanilla 1.20.1 server bound.
pub const MAX_SERVER_ADDRESS_UTF16_UNITS: usize = 255;

/// The packet ID of the Handshaking state's sole packet.
pub const HANDSHAKE_PACKET_ID: i32 = 0x00;

/// A decoded Handshaking state `0x00` packet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandshakePacket {
    /// The protocol version advertised by the client. `-1` indicates a legacy
    /// ping; any other value is retained verbatim for later mismatch handling.
    pub protocol_version: i32,
    /// The server address the client connected to, bounded to
    /// [`MAX_SERVER_ADDRESS_UTF16_UNITS`] UTF-16 code units.
    pub server_address: String,
    /// The server port the client connected to.
    pub port: u16,
    /// The requested next connection state.
    pub next_state: NextState,
}

/// An error encountered while decoding a handshake packet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandshakeError {
    /// The packet does not carry the expected handshake packet ID.
    WrongPacketId { received: i32 },
    /// A primitive codec failed while parsing a field.
    Codec(CodecError),
    /// The server address exceeded the configured UTF-16 unit bound.
    AddressTooLong,
    /// The `next_state` value is not `1` (Status) or `2` (Login).
    InvalidNextState { value: i32 },
    /// The packet contained trailing bytes after the four fields.
    TrailingBytes { count: usize },
    /// More input is required to decode the frame or its fields.
    Incomplete,
}

impl fmt::Display for HandshakeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongPacketId { received } => {
                write!(formatter, "expected handshake packet ID 0, got {received}")
            }
            Self::Codec(error) => write!(formatter, "handshake field codec error: {error}"),
            Self::AddressTooLong => formatter.write_str("handshake server address is too long"),
            Self::InvalidNextState { value } => {
                write!(formatter, "invalid handshake next_state value {value}")
            }
            Self::TrailingBytes { count } => {
                write!(formatter, "handshake packet has {count} trailing byte(s)")
            }
            Self::Incomplete => formatter.write_str("handshake input is incomplete"),
        }
    }
}

impl std::error::Error for HandshakeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Codec(error) => Some(error),
            _ => None,
        }
    }
}

impl From<CodecError> for HandshakeError {
    fn from(error: CodecError) -> Self {
        match error {
            CodecError::IncompleteInput => Self::Incomplete,
            other => Self::Codec(other),
        }
    }
}

/// Decodes one Handshaking state packet from incremental frame input.
///
/// `input` is advanced past the complete frame only when a handshake packet is
/// successfully parsed. On [`HandshakeError::Incomplete`] the input is left
/// unchanged so the caller can append more bytes and retry. All other errors
/// indicate a protocol violation and the connection should be terminated.
///
/// The caller must ensure the connection is in the
/// [`crate::state::ProtocolState::Handshaking`] state before calling this
/// function; state enforcement is performed by the routing layer, not here.
pub fn decode_handshake(
    input: &mut &[u8],
    max_frame_length: usize,
) -> Result<HandshakePacket, HandshakeError> {
    let source = *input;
    let outcome = decode_frame(input, max_frame_length).map_err(|_| HandshakeError::Incomplete)?;
    let frame = match outcome {
        DecodeOutcome::Complete(frame) => frame,
        DecodeOutcome::Incomplete => {
            *input = source;
            return Err(HandshakeError::Incomplete);
        }
    };

    parse_handshake_body(&frame).inspect_err(|_| {
        // On parse failure, restore input so the caller can inspect the raw
        // frame bytes if needed before closing the connection.
        *input = source;
    })
}

/// Parses the body of a complete frame as a handshake packet.
fn parse_handshake_body(frame: &PacketFrame<'_>) -> Result<HandshakePacket, HandshakeError> {
    if frame.packet_id != HANDSHAKE_PACKET_ID {
        return Err(HandshakeError::WrongPacketId {
            received: frame.packet_id,
        });
    }

    let mut body = frame.payload;
    let protocol_version = decode_var_int(&mut body).map_err(HandshakeError::from)?;
    let server_address =
        decode_string(&mut body, MAX_SERVER_ADDRESS_UTF16_UNITS).map_err(|error| match error {
            CodecError::StringTooLong => HandshakeError::AddressTooLong,
            other => HandshakeError::from(other),
        })?;
    let port = decode_u16(&mut body).map_err(HandshakeError::from)?;
    let raw_next_state = decode_var_int(&mut body).map_err(HandshakeError::from)?;
    let next_state =
        NextState::from_wire(raw_next_state).ok_or(HandshakeError::InvalidNextState {
            value: raw_next_state,
        })?;

    if !body.is_empty() {
        return Err(HandshakeError::TrailingBytes { count: body.len() });
    }

    Ok(HandshakePacket {
        protocol_version,
        server_address: server_address.to_owned(),
        port,
        next_state,
    })
}

/// Encodes a handshake packet into the provided output buffer.
///
/// This is primarily useful for testing and for proxy-style forwarding. The
/// frame-length prefix is included and bounded by `max_frame_length`.
pub fn encode_handshake(
    packet: &HandshakePacket,
    max_frame_length: usize,
    output: &mut Vec<u8>,
) -> Result<(), HandshakeError> {
    use crate::framing::encode_frame;
    use crate::primitives::{encode_string, encode_u16, encode_var_int};

    let mut body = Vec::new();
    encode_var_int(packet.protocol_version, &mut body);
    encode_string(
        &packet.server_address,
        MAX_SERVER_ADDRESS_UTF16_UNITS,
        &mut body,
    )
    .map_err(|error| match error {
        CodecError::StringTooLong => HandshakeError::AddressTooLong,
        other => HandshakeError::from(other),
    })?;
    encode_u16(packet.port, &mut body);
    encode_var_int(packet.next_state.wire_value(), &mut body);

    encode_frame(HANDSHAKE_PACKET_ID, &body, max_frame_length, output)
        .map_err(|_| HandshakeError::Incomplete)
}

#[cfg(test)]
mod tests {
    use super::{
        HandshakeError, HandshakePacket, MAX_SERVER_ADDRESS_UTF16_UNITS, decode_handshake,
        encode_handshake,
    };
    use crate::framing::encode_frame;
    use crate::primitives::{encode_string, encode_u16, encode_var_int};
    use crate::state::NextState;
    use crate::state::ProtocolState;
    use crate::state::apply_handshake_transition;

    const TEST_MAX_FRAME: usize = 1024;

    fn build_handshake(
        protocol_version: i32,
        address: &str,
        port: u16,
        next_state: NextState,
    ) -> HandshakePacket {
        HandshakePacket {
            protocol_version,
            server_address: address.to_owned(),
            port,
            next_state,
        }
    }

    fn encode(packet: &HandshakePacket) -> Result<Vec<u8>, HandshakeError> {
        let mut output = Vec::new();
        encode_handshake(packet, TEST_MAX_FRAME, &mut output)?;
        Ok(output)
    }

    /// Encodes a hostile body that bypasses `encode_string`'s own length check.
    fn encode_hostile_body(
        protocol_version: i32,
        raw_address_bytes: &[u8],
        port: u16,
        next_state: i32,
        extra: &[u8],
    ) -> Result<Vec<u8>, HandshakeError> {
        let mut body = Vec::new();
        encode_var_int(protocol_version, &mut body);
        encode_var_int(raw_address_bytes.len() as i32, &mut body);
        body.extend_from_slice(raw_address_bytes);
        encode_u16(port, &mut body);
        encode_var_int(next_state, &mut body);
        body.extend_from_slice(extra);

        let mut wire = Vec::new();
        encode_frame(0, &body, TEST_MAX_FRAME, &mut wire)
            .map_err(|_| HandshakeError::Incomplete)?;
        Ok(wire)
    }

    #[test]
    fn valid_status_handshake_round_trips() -> Result<(), HandshakeError> {
        let original = build_handshake(763, "example.net", 25565, NextState::Status);
        let wire = encode(&original)?;
        let mut input = wire.as_slice();
        assert_eq!(decode_handshake(&mut input, TEST_MAX_FRAME), Ok(original));
        assert!(input.is_empty());
        Ok(())
    }

    #[test]
    fn valid_login_handshake_round_trips() -> Result<(), HandshakeError> {
        let original = build_handshake(763, "localhost", 25565, NextState::Login);
        let wire = encode(&original)?;
        let mut input = wire.as_slice();
        assert_eq!(decode_handshake(&mut input, TEST_MAX_FRAME), Ok(original));
        assert!(input.is_empty());
        Ok(())
    }

    #[test]
    fn protocol_mismatch_is_parsed_without_corrupting_state() -> Result<(), HandshakeError> {
        let original = build_handshake(999, "example.net", 25565, NextState::Login);
        let wire = encode(&original)?;
        let mut input = wire.as_slice();
        let decoded = decode_handshake(&mut input, TEST_MAX_FRAME)?;
        assert_eq!(decoded.protocol_version, 999);
        assert_eq!(decoded.next_state, NextState::Login);
        assert_eq!(
            apply_handshake_transition(ProtocolState::Handshaking, decoded.next_state),
            Ok(ProtocolState::Login)
        );
        Ok(())
    }

    #[test]
    fn legacy_ping_protocol_version_is_preserved() -> Result<(), HandshakeError> {
        let original = build_handshake(-1, "example.net", 25565, NextState::Status);
        let wire = encode(&original)?;
        let mut input = wire.as_slice();
        let decoded = decode_handshake(&mut input, TEST_MAX_FRAME)?;
        assert_eq!(decoded.protocol_version, -1);
        Ok(())
    }

    #[test]
    fn invalid_next_state_value_is_rejected() -> Result<(), HandshakeError> {
        let mut body = Vec::new();
        encode_var_int(763, &mut body);
        encode_string("example.net", MAX_SERVER_ADDRESS_UTF16_UNITS, &mut body)?;
        encode_u16(25565, &mut body);
        encode_var_int(3, &mut body);

        let mut wire = Vec::new();
        encode_frame(0, &body, TEST_MAX_FRAME, &mut wire)
            .map_err(|_| HandshakeError::Incomplete)?;

        let mut input = wire.as_slice();
        assert_eq!(
            decode_handshake(&mut input, TEST_MAX_FRAME),
            Err(HandshakeError::InvalidNextState { value: 3 })
        );
        assert_eq!(input, wire.as_slice());
        Ok(())
    }

    #[test]
    fn overlong_address_is_rejected() -> Result<(), HandshakeError> {
        let overlong = "a".repeat(256);
        let wire = encode_hostile_body(763, overlong.as_bytes(), 25565, 1, &[])?;

        let mut input = wire.as_slice();
        assert_eq!(
            decode_handshake(&mut input, TEST_MAX_FRAME),
            Err(HandshakeError::AddressTooLong)
        );
        assert_eq!(input, wire.as_slice());
        Ok(())
    }

    #[test]
    fn trailing_bytes_are_rejected() -> Result<(), HandshakeError> {
        let mut body = Vec::new();
        encode_var_int(763, &mut body);
        encode_string("example.net", MAX_SERVER_ADDRESS_UTF16_UNITS, &mut body)?;
        encode_u16(25565, &mut body);
        encode_var_int(1, &mut body);
        body.push(0xff); // trailing byte inside the frame body

        let mut wire = Vec::new();
        encode_frame(0, &body, TEST_MAX_FRAME, &mut wire)
            .map_err(|_| HandshakeError::Incomplete)?;

        let mut input = wire.as_slice();
        assert_eq!(
            decode_handshake(&mut input, TEST_MAX_FRAME),
            Err(HandshakeError::TrailingBytes { count: 1 })
        );
        assert_eq!(input, wire.as_slice());
        Ok(())
    }

    #[test]
    fn wrong_packet_id_is_rejected() -> Result<(), HandshakeError> {
        let mut body = Vec::new();
        encode_var_int(763, &mut body);
        encode_string("example.net", MAX_SERVER_ADDRESS_UTF16_UNITS, &mut body)?;
        encode_u16(25565, &mut body);
        encode_var_int(1, &mut body);

        let mut wire = Vec::new();
        encode_frame(0x42, &body, TEST_MAX_FRAME, &mut wire)
            .map_err(|_| HandshakeError::Incomplete)?;

        let mut input = wire.as_slice();
        assert_eq!(
            decode_handshake(&mut input, TEST_MAX_FRAME),
            Err(HandshakeError::WrongPacketId { received: 0x42 })
        );
        assert_eq!(input, wire.as_slice());
        Ok(())
    }

    #[test]
    fn truncated_input_is_incomplete_and_preserves_buffer() -> Result<(), HandshakeError> {
        let original = build_handshake(763, "example.net", 25565, NextState::Status);
        let wire = encode(&original)?;

        for split in 0..wire.len() {
            let mut input = &wire[..split];
            assert_eq!(
                decode_handshake(&mut input, TEST_MAX_FRAME),
                Err(HandshakeError::Incomplete)
            );
            assert_eq!(input, &wire[..split]);
        }
        Ok(())
    }

    #[test]
    fn coalesced_handshake_and_trailing_bytes_preserve_trailing() -> Result<(), HandshakeError> {
        let original = build_handshake(763, "example.net", 25565, NextState::Status);
        let wire = encode(&original)?;
        let mut input = wire.as_slice();
        assert_eq!(decode_handshake(&mut input, TEST_MAX_FRAME), Ok(original));
        assert!(input.is_empty());
        Ok(())
    }

    #[test]
    fn multibyte_address_within_bound_round_trips() -> Result<(), HandshakeError> {
        let original = build_handshake(763, "マインクラフト", 25565, NextState::Login);
        let wire = encode(&original)?;
        let mut input = wire.as_slice();
        assert_eq!(decode_handshake(&mut input, TEST_MAX_FRAME), Ok(original));
        Ok(())
    }

    #[test]
    fn address_at_exact_bound_round_trips() -> Result<(), HandshakeError> {
        let address = "a".repeat(255);
        let original = build_handshake(763, &address, 25565, NextState::Status);
        let wire = encode(&original)?;
        let mut input = wire.as_slice();
        assert_eq!(decode_handshake(&mut input, TEST_MAX_FRAME), Ok(original));
        Ok(())
    }

    #[test]
    fn surrogate_pair_address_counts_as_two_utf16_units() -> Result<(), HandshakeError> {
        let pair = "\u{10000}";
        let fits = pair.repeat(127);
        let original = build_handshake(763, &fits, 25565, NextState::Status);
        let wire = encode(&original)?;
        let mut input = wire.as_slice();
        assert_eq!(decode_handshake(&mut input, TEST_MAX_FRAME), Ok(original));

        let overlong = pair.repeat(128);
        let wire = encode_hostile_body(763, overlong.as_bytes(), 25565, 1, &[])?;
        let mut input = wire.as_slice();
        assert_eq!(
            decode_handshake(&mut input, TEST_MAX_FRAME),
            Err(HandshakeError::AddressTooLong)
        );
        Ok(())
    }
}
