//! Login state packet codecs for protocol 763 (Minecraft Java Edition 1.20.1).
//!
//! This module implements the Login Start (serverbound `0x00`), Login
//! Disconnect (clientbound `0x00`), Encryption Request (clientbound `0x01`),
//! and Encryption Response (serverbound `0x01`) packets.

use std::fmt;

use crate::framing::{DecodeOutcome, FramingError, decode_frame, encode_frame};
use crate::primitives::{
    CodecError, MAX_CHAT_COMPONENT_LENGTH, MAX_PUBLIC_KEY_LENGTH, MAX_SERVER_ID_LENGTH,
    MAX_USERNAME_LENGTH, MAX_VERIFY_TOKEN_LENGTH, Uuid, decode_byte_array, decode_string,
    decode_uuid, encode_byte_array, encode_string, encode_uuid,
};
use crate::state::ProtocolState;

/// Packet ID for the serverbound Login Start packet.
pub const LOGIN_START_PACKET_ID: i32 = 0x00;

/// Packet ID for the clientbound Login Disconnect packet.
pub const LOGIN_DISCONNECT_PACKET_ID: i32 = 0x00;

/// Packet ID for the clientbound Encryption Request packet.
pub const ENCRYPTION_REQUEST_PACKET_ID: i32 = 0x01;

/// Packet ID for the serverbound Encryption Response packet.
pub const ENCRYPTION_RESPONSE_PACKET_ID: i32 = 0x01;

/// An error encountered while encoding or decoding a login packet.
#[derive(Debug)]
pub enum LoginError {
    /// A framing error occurred.
    Framing(FramingError),
    /// A primitive codec error occurred.
    Codec(CodecError),
    /// The packet ID does not match the expected value.
    WrongPacketId { received: i32, expected: i32 },
    /// The packet contained trailing bytes after the expected fields.
    TrailingBytes { count: usize },
    /// The connection is not in the Login state.
    WrongState { received: ProtocolState },
    /// More input is required.
    Incomplete,
}

impl fmt::Display for LoginError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Framing(error) => write!(formatter, "framing error: {error}"),
            Self::Codec(error) => write!(formatter, "codec error: {error}"),
            Self::WrongPacketId { received, expected } => {
                write!(formatter, "expected packet ID {expected}, got {received}")
            }
            Self::TrailingBytes { count } => {
                write!(formatter, "packet has {count} trailing byte(s)")
            }
            Self::WrongState { received } => {
                write!(formatter, "expected Login state, got {received:?}")
            }
            Self::Incomplete => formatter.write_str("incomplete input"),
        }
    }
}

impl std::error::Error for LoginError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Framing(error) => Some(error),
            Self::Codec(error) => Some(error),
            _ => None,
        }
    }
}

impl From<FramingError> for LoginError {
    fn from(error: FramingError) -> Self {
        match error {
            FramingError::LengthCodec(CodecError::IncompleteInput)
            | FramingError::PacketIdCodec(CodecError::IncompleteInput) => Self::Incomplete,
            other => Self::Framing(other),
        }
    }
}

impl From<CodecError> for LoginError {
    fn from(error: CodecError) -> Self {
        match error {
            CodecError::IncompleteInput => Self::Incomplete,
            other => Self::Codec(other),
        }
    }
}

/// The result of attempting to decode one login packet.
#[derive(Debug, PartialEq, Eq)]
pub enum LoginDecodeOutcome {
    /// One complete packet was consumed.
    Complete(LoginPacket),
    /// More bytes are required and no input was consumed.
    Incomplete,
}

/// A decoded login packet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginPacket {
    /// Serverbound Login Start (`0x00`).
    LoginStart(LoginStart),
    /// Clientbound Login Disconnect (`0x00`).
    LoginDisconnect(LoginDisconnect),
    /// Clientbound Encryption Request (`0x01`).
    EncryptionRequest(EncryptionRequest),
    /// Serverbound Encryption Response (`0x01`).
    EncryptionResponse(EncryptionResponse),
}

/// Serverbound Login Start packet (Login `0x00`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginStart {
    /// The player's username (max 16 UTF-16 units).
    pub username: String,
    /// The player's UUID.
    pub uuid: Uuid,
}

/// Clientbound Login Disconnect packet (Login `0x00`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginDisconnect {
    /// The disconnect reason as a JSON chat component string.
    pub reason: String,
}

/// Clientbound Encryption Request packet (Login `0x01`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncryptionRequest {
    /// The server ID (max 20 UTF-16 units, may be empty).
    pub server_id: String,
    /// The server's public key (max 512 bytes).
    pub public_key: Vec<u8>,
    /// The verify token (max 16 bytes).
    pub verify_token: Vec<u8>,
}

/// Serverbound Encryption Response packet (Login `0x01`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncryptionResponse {
    /// The encrypted shared secret (must be 16 bytes for AES-128).
    pub shared_secret: Vec<u8>,
    /// The encrypted verify token (max 16 bytes).
    pub verify_token: Vec<u8>,
}

/// Encodes a Login Start packet (serverbound Login `0x00`).
///
/// On error, `output` is unchanged.
pub fn encode_login_start(
    packet: &LoginStart,
    max_frame_length: usize,
    output: &mut Vec<u8>,
) -> Result<(), LoginError> {
    let mut body = Vec::new();
    encode_string(&packet.username, MAX_USERNAME_LENGTH, &mut body).map_err(LoginError::from)?;
    encode_uuid(packet.uuid, &mut body);

    encode_frame(LOGIN_START_PACKET_ID, &body, max_frame_length, output)
        .map_err(LoginError::from)?;
    Ok(())
}

/// Decodes a Login Start packet (serverbound Login `0x00`).
///
/// On [`LoginDecodeOutcome::Incomplete`], the input is unchanged.
pub fn decode_login_start(
    input: &mut &[u8],
    max_frame_length: usize,
) -> Result<LoginDecodeOutcome, LoginError> {
    let source = *input;
    let frame = match decode_frame(input, max_frame_length) {
        Ok(DecodeOutcome::Complete(frame)) => frame,
        Ok(DecodeOutcome::Incomplete) => {
            *input = source;
            return Ok(LoginDecodeOutcome::Incomplete);
        }
        Err(error) => {
            *input = source;
            return Err(LoginError::from(error));
        }
    };

    if frame.packet_id != LOGIN_START_PACKET_ID {
        *input = source;
        return Err(LoginError::WrongPacketId {
            received: frame.packet_id,
            expected: LOGIN_START_PACKET_ID,
        });
    }

    let mut body = frame.payload;
    let username = decode_string(&mut body, MAX_USERNAME_LENGTH).map_err(|error| {
        *input = source;
        LoginError::from(error)
    })?;
    let uuid = decode_uuid(&mut body).map_err(|error| {
        *input = source;
        LoginError::from(error)
    })?;

    if !body.is_empty() {
        *input = source;
        return Err(LoginError::TrailingBytes { count: body.len() });
    }

    Ok(LoginDecodeOutcome::Complete(LoginPacket::LoginStart(
        LoginStart {
            username: username.to_string(),
            uuid,
        },
    )))
}

/// Encodes a Login Disconnect packet (clientbound Login `0x00`).
///
/// On error, `output` is unchanged.
pub fn encode_login_disconnect(
    packet: &LoginDisconnect,
    max_frame_length: usize,
    output: &mut Vec<u8>,
) -> Result<(), LoginError> {
    let mut body = Vec::new();
    encode_string(&packet.reason, MAX_CHAT_COMPONENT_LENGTH, &mut body)
        .map_err(LoginError::from)?;

    encode_frame(LOGIN_DISCONNECT_PACKET_ID, &body, max_frame_length, output)
        .map_err(LoginError::from)?;
    Ok(())
}

/// Decodes a Login Disconnect packet (clientbound Login `0x00`).
///
/// On [`LoginDecodeOutcome::Incomplete`], the input is unchanged.
pub fn decode_login_disconnect(
    input: &mut &[u8],
    max_frame_length: usize,
) -> Result<LoginDecodeOutcome, LoginError> {
    let source = *input;
    let frame = match decode_frame(input, max_frame_length) {
        Ok(DecodeOutcome::Complete(frame)) => frame,
        Ok(DecodeOutcome::Incomplete) => {
            *input = source;
            return Ok(LoginDecodeOutcome::Incomplete);
        }
        Err(error) => {
            *input = source;
            return Err(LoginError::from(error));
        }
    };

    if frame.packet_id != LOGIN_DISCONNECT_PACKET_ID {
        *input = source;
        return Err(LoginError::WrongPacketId {
            received: frame.packet_id,
            expected: LOGIN_DISCONNECT_PACKET_ID,
        });
    }

    let mut body = frame.payload;
    let reason = decode_string(&mut body, MAX_CHAT_COMPONENT_LENGTH).map_err(|error| {
        *input = source;
        LoginError::from(error)
    })?;

    if !body.is_empty() {
        *input = source;
        return Err(LoginError::TrailingBytes { count: body.len() });
    }

    Ok(LoginDecodeOutcome::Complete(LoginPacket::LoginDisconnect(
        LoginDisconnect {
            reason: reason.to_string(),
        },
    )))
}

/// Decodes any login packet from incremental input.
///
/// This dispatches based on the packet ID. Since both Login Start and Login
/// Disconnect share packet ID `0x00`, the `direction` parameter disambiguates
/// them.
///
/// On [`LoginDecodeOutcome::Incomplete`], the input is unchanged.
pub fn decode_login_packet(
    input: &mut &[u8],
    direction: LoginDirection,
    max_frame_length: usize,
) -> Result<LoginDecodeOutcome, LoginError> {
    match direction {
        LoginDirection::Serverbound => decode_login_start(input, max_frame_length),
        LoginDirection::Clientbound => decode_login_disconnect(input, max_frame_length),
    }
}

/// The direction of a login packet, used to disambiguate packet ID `0x00`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginDirection {
    /// Serverbound (client to server): Login Start.
    Serverbound,
    /// Clientbound (server to client): Login Disconnect.
    Clientbound,
}

/// Verifies that the connection is in the Login state.
pub fn ensure_login_state(state: ProtocolState) -> Result<(), LoginError> {
    match state {
        ProtocolState::Login => Ok(()),
        other => Err(LoginError::WrongState { received: other }),
    }
}

/// Encodes an Encryption Request packet (clientbound Login `0x01`).
///
/// On error, `output` is unchanged.
pub fn encode_encryption_request(
    packet: &EncryptionRequest,
    max_frame_length: usize,
    output: &mut Vec<u8>,
) -> Result<(), LoginError> {
    let mut body = Vec::new();
    encode_string(&packet.server_id, MAX_SERVER_ID_LENGTH, &mut body).map_err(LoginError::from)?;
    encode_byte_array(&packet.public_key, MAX_PUBLIC_KEY_LENGTH, &mut body)
        .map_err(LoginError::from)?;
    encode_byte_array(&packet.verify_token, MAX_VERIFY_TOKEN_LENGTH, &mut body)
        .map_err(LoginError::from)?;

    encode_frame(
        ENCRYPTION_REQUEST_PACKET_ID,
        &body,
        max_frame_length,
        output,
    )
    .map_err(LoginError::from)?;
    Ok(())
}

/// Decodes an Encryption Request packet (clientbound Login `0x01`).
///
/// On [`LoginDecodeOutcome::Incomplete`], the input is unchanged.
pub fn decode_encryption_request(
    input: &mut &[u8],
    max_frame_length: usize,
) -> Result<LoginDecodeOutcome, LoginError> {
    let source = *input;
    let frame = match decode_frame(input, max_frame_length) {
        Ok(DecodeOutcome::Complete(frame)) => frame,
        Ok(DecodeOutcome::Incomplete) => {
            *input = source;
            return Ok(LoginDecodeOutcome::Incomplete);
        }
        Err(error) => {
            *input = source;
            return Err(LoginError::from(error));
        }
    };

    if frame.packet_id != ENCRYPTION_REQUEST_PACKET_ID {
        *input = source;
        return Err(LoginError::WrongPacketId {
            received: frame.packet_id,
            expected: ENCRYPTION_REQUEST_PACKET_ID,
        });
    }

    let mut body = frame.payload;
    let server_id = decode_string(&mut body, MAX_SERVER_ID_LENGTH).map_err(|error| {
        *input = source;
        LoginError::from(error)
    })?;
    let public_key = decode_byte_array(&mut body, MAX_PUBLIC_KEY_LENGTH).map_err(|error| {
        *input = source;
        LoginError::from(error)
    })?;
    let verify_token = decode_byte_array(&mut body, MAX_VERIFY_TOKEN_LENGTH).map_err(|error| {
        *input = source;
        LoginError::from(error)
    })?;

    if !body.is_empty() {
        *input = source;
        return Err(LoginError::TrailingBytes { count: body.len() });
    }

    Ok(LoginDecodeOutcome::Complete(
        LoginPacket::EncryptionRequest(EncryptionRequest {
            server_id: server_id.to_string(),
            public_key: public_key.to_vec(),
            verify_token: verify_token.to_vec(),
        }),
    ))
}

/// Encodes an Encryption Response packet (serverbound Login `0x01`).
///
/// On error, `output` is unchanged.
pub fn encode_encryption_response(
    packet: &EncryptionResponse,
    max_frame_length: usize,
    output: &mut Vec<u8>,
) -> Result<(), LoginError> {
    let mut body = Vec::new();
    encode_byte_array(&packet.shared_secret, MAX_PUBLIC_KEY_LENGTH, &mut body)
        .map_err(LoginError::from)?;
    encode_byte_array(&packet.verify_token, MAX_VERIFY_TOKEN_LENGTH, &mut body)
        .map_err(LoginError::from)?;

    encode_frame(
        ENCRYPTION_RESPONSE_PACKET_ID,
        &body,
        max_frame_length,
        output,
    )
    .map_err(LoginError::from)?;
    Ok(())
}

/// Decodes an Encryption Response packet (serverbound Login `0x01`).
///
/// On [`LoginDecodeOutcome::Incomplete`], the input is unchanged.
pub fn decode_encryption_response(
    input: &mut &[u8],
    max_frame_length: usize,
) -> Result<LoginDecodeOutcome, LoginError> {
    let source = *input;
    let frame = match decode_frame(input, max_frame_length) {
        Ok(DecodeOutcome::Complete(frame)) => frame,
        Ok(DecodeOutcome::Incomplete) => {
            *input = source;
            return Ok(LoginDecodeOutcome::Incomplete);
        }
        Err(error) => {
            *input = source;
            return Err(LoginError::from(error));
        }
    };

    if frame.packet_id != ENCRYPTION_RESPONSE_PACKET_ID {
        *input = source;
        return Err(LoginError::WrongPacketId {
            received: frame.packet_id,
            expected: ENCRYPTION_RESPONSE_PACKET_ID,
        });
    }

    let mut body = frame.payload;
    let shared_secret = decode_byte_array(&mut body, MAX_PUBLIC_KEY_LENGTH).map_err(|error| {
        *input = source;
        LoginError::from(error)
    })?;
    let verify_token = decode_byte_array(&mut body, MAX_VERIFY_TOKEN_LENGTH).map_err(|error| {
        *input = source;
        LoginError::from(error)
    })?;

    if !body.is_empty() {
        *input = source;
        return Err(LoginError::TrailingBytes { count: body.len() });
    }

    Ok(LoginDecodeOutcome::Complete(
        LoginPacket::EncryptionResponse(EncryptionResponse {
            shared_secret: shared_secret.to_vec(),
            verify_token: verify_token.to_vec(),
        }),
    ))
}

#[cfg(test)]
mod tests {
    use super::{
        ENCRYPTION_REQUEST_PACKET_ID, ENCRYPTION_RESPONSE_PACKET_ID, EncryptionRequest,
        EncryptionResponse, LOGIN_START_PACKET_ID, LoginDirection, LoginDisconnect, LoginError,
        LoginPacket, LoginStart, MAX_PUBLIC_KEY_LENGTH, MAX_SERVER_ID_LENGTH,
        MAX_VERIFY_TOKEN_LENGTH, decode_encryption_request, decode_encryption_response,
        decode_login_disconnect, decode_login_packet, decode_login_start,
        encode_encryption_request, encode_encryption_response, encode_login_disconnect,
        encode_login_start, ensure_login_state,
    };
    use crate::primitives::{SHARED_SECRET_LENGTH, Uuid};
    use crate::state::ProtocolState;

    const TEST_MAX_FRAME: usize = 65536;

    fn sample_uuid() -> Uuid {
        Uuid::from_be_bytes([
            0x06, 0x9a, 0x64, 0x8f, 0x86, 0x4c, 0x4e, 0x47, 0xa1, 0x0b, 0x6c, 0xd3, 0x8b, 0x6e,
            0x4c, 0x21,
        ])
    }

    #[test]
    fn login_start_round_trips() -> Result<(), LoginError> {
        let packet = LoginStart {
            username: "Steve".to_string(),
            uuid: sample_uuid(),
        };
        let mut wire = Vec::new();
        encode_login_start(&packet, TEST_MAX_FRAME, &mut wire)?;

        let mut input = wire.as_slice();
        match decode_login_start(&mut input, TEST_MAX_FRAME)? {
            super::LoginDecodeOutcome::Complete(LoginPacket::LoginStart(decoded)) => {
                assert_eq!(decoded, packet);
            }
            _ => panic!("expected LoginStart"),
        }
        assert!(input.is_empty());
        Ok(())
    }

    #[test]
    fn login_start_at_exact_16_char_bound() -> Result<(), LoginError> {
        let username = "A".repeat(16);
        let packet = LoginStart {
            username,
            uuid: sample_uuid(),
        };
        let mut wire = Vec::new();
        encode_login_start(&packet, TEST_MAX_FRAME, &mut wire)?;

        let mut input = wire.as_slice();
        match decode_login_start(&mut input, TEST_MAX_FRAME)? {
            super::LoginDecodeOutcome::Complete(LoginPacket::LoginStart(decoded)) => {
                assert_eq!(decoded.username, packet.username)
            }
            _ => panic!("expected LoginStart"),
        }
        Ok(())
    }

    #[test]
    fn login_start_overlong_username_is_rejected() -> Result<(), LoginError> {
        let username = "A".repeat(17);
        let packet = LoginStart {
            username,
            uuid: sample_uuid(),
        };
        let result = encode_login_start(&packet, TEST_MAX_FRAME, &mut Vec::new());
        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn login_start_with_multibyte_username_at_bound() -> Result<(), LoginError> {
        // 8 surrogate pairs = 16 UTF-16 units
        let username: String = "𝕏".repeat(8);
        assert_eq!(username.chars().count(), 8);
        let packet = LoginStart {
            username,
            uuid: sample_uuid(),
        };
        let mut wire = Vec::new();
        encode_login_start(&packet, TEST_MAX_FRAME, &mut wire)?;

        let mut input = wire.as_slice();
        match decode_login_start(&mut input, TEST_MAX_FRAME)? {
            super::LoginDecodeOutcome::Complete(LoginPacket::LoginStart(decoded)) => {
                assert_eq!(decoded.username, packet.username)
            }
            _ => panic!("expected LoginStart"),
        }
        Ok(())
    }

    #[test]
    fn login_start_wrong_packet_id_is_rejected() -> Result<(), LoginError> {
        let packet = LoginStart {
            username: "Steve".to_string(),
            uuid: sample_uuid(),
        };
        let mut body = Vec::new();
        crate::primitives::encode_string(&packet.username, 16, &mut body)
            .map_err(LoginError::from)?;
        crate::primitives::encode_uuid(packet.uuid, &mut body);

        let mut wire = Vec::new();
        crate::framing::encode_frame(0x05, &body, TEST_MAX_FRAME, &mut wire)
            .map_err(LoginError::from)?;

        let mut input = wire.as_slice();
        let result = decode_login_start(&mut input, TEST_MAX_FRAME);
        assert!(matches!(result, Err(LoginError::WrongPacketId { .. })));
        assert_eq!(input, wire.as_slice());
        Ok(())
    }

    #[test]
    fn login_start_truncated_is_incomplete() -> Result<(), LoginError> {
        let packet = LoginStart {
            username: "Steve".to_string(),
            uuid: sample_uuid(),
        };
        let mut wire = Vec::new();
        encode_login_start(&packet, TEST_MAX_FRAME, &mut wire)?;

        for split in 0..wire.len() {
            let mut input = &wire[..split];
            assert_eq!(
                decode_login_start(&mut input, TEST_MAX_FRAME)?,
                super::LoginDecodeOutcome::Incomplete
            );
            assert_eq!(input, &wire[..split]);
        }
        Ok(())
    }

    #[test]
    fn login_start_trailing_bytes_are_rejected() -> Result<(), LoginError> {
        let packet = LoginStart {
            username: "Steve".to_string(),
            uuid: sample_uuid(),
        };
        let mut body = Vec::new();
        crate::primitives::encode_string(&packet.username, 16, &mut body)
            .map_err(LoginError::from)?;
        crate::primitives::encode_uuid(packet.uuid, &mut body);
        body.push(0xff); // trailing byte

        let mut wire = Vec::new();
        crate::framing::encode_frame(LOGIN_START_PACKET_ID, &body, TEST_MAX_FRAME, &mut wire)
            .map_err(LoginError::from)?;

        let mut input = wire.as_slice();
        let result = decode_login_start(&mut input, TEST_MAX_FRAME);
        assert!(matches!(result, Err(LoginError::TrailingBytes { .. })));
        Ok(())
    }

    #[test]
    fn login_disconnect_round_trips_plain_text() -> Result<(), LoginError> {
        let packet = LoginDisconnect {
            reason: r#"{"text":"Server is full"}"#.to_string(),
        };
        let mut wire = Vec::new();
        encode_login_disconnect(&packet, TEST_MAX_FRAME, &mut wire)?;

        let mut input = wire.as_slice();
        match decode_login_disconnect(&mut input, TEST_MAX_FRAME)? {
            super::LoginDecodeOutcome::Complete(LoginPacket::LoginDisconnect(decoded)) => {
                assert_eq!(decoded, packet)
            }
            _ => panic!("expected LoginDisconnect"),
        }
        assert!(input.is_empty());
        Ok(())
    }

    #[test]
    fn login_disconnect_round_trips_component_json() -> Result<(), LoginError> {
        let packet = LoginDisconnect {
            reason: r#"{"translate":"multiplayer.disconnect.kicked"}"#.to_string(),
        };
        let mut wire = Vec::new();
        encode_login_disconnect(&packet, TEST_MAX_FRAME, &mut wire)?;

        let mut input = wire.as_slice();
        match decode_login_disconnect(&mut input, TEST_MAX_FRAME)? {
            super::LoginDecodeOutcome::Complete(LoginPacket::LoginDisconnect(decoded)) => {
                assert_eq!(decoded, packet)
            }
            _ => panic!("expected LoginDisconnect"),
        }
        Ok(())
    }

    #[test]
    fn login_disconnect_wrong_packet_id_is_rejected() -> Result<(), LoginError> {
        let mut body = Vec::new();
        crate::primitives::encode_string(r#"{"text":"bye"}"#, 32767, &mut body)
            .map_err(LoginError::from)?;
        let mut wire = Vec::new();
        crate::framing::encode_frame(0x05, &body, TEST_MAX_FRAME, &mut wire)
            .map_err(LoginError::from)?;

        let mut input = wire.as_slice();
        let result = decode_login_disconnect(&mut input, TEST_MAX_FRAME);
        assert!(matches!(result, Err(LoginError::WrongPacketId { .. })));
        Ok(())
    }

    #[test]
    fn login_disconnect_truncated_is_incomplete() -> Result<(), LoginError> {
        let packet = LoginDisconnect {
            reason: r#"{"text":"bye"}"#.to_string(),
        };
        let mut wire = Vec::new();
        encode_login_disconnect(&packet, TEST_MAX_FRAME, &mut wire)?;

        for split in 0..wire.len() {
            let mut input = &wire[..split];
            assert_eq!(
                decode_login_disconnect(&mut input, TEST_MAX_FRAME)?,
                super::LoginDecodeOutcome::Incomplete
            );
            assert_eq!(input, &wire[..split]);
        }
        Ok(())
    }

    #[test]
    fn login_disconnect_trailing_bytes_are_rejected() -> Result<(), LoginError> {
        let mut body = Vec::new();
        crate::primitives::encode_string(r#"{"text":"bye"}"#, 32767, &mut body)
            .map_err(LoginError::from)?;
        body.push(0xff);

        let mut wire = Vec::new();
        crate::framing::encode_frame(
            super::LOGIN_DISCONNECT_PACKET_ID,
            &body,
            TEST_MAX_FRAME,
            &mut wire,
        )
        .map_err(LoginError::from)?;

        let mut input = wire.as_slice();
        let result = decode_login_disconnect(&mut input, TEST_MAX_FRAME);
        assert!(matches!(result, Err(LoginError::TrailingBytes { .. })));
        Ok(())
    }

    #[test]
    fn decode_login_packet_dispatches_by_direction() -> Result<(), LoginError> {
        let start = LoginStart {
            username: "Alex".to_string(),
            uuid: sample_uuid(),
        };
        let mut wire = Vec::new();
        encode_login_start(&start, TEST_MAX_FRAME, &mut wire)?;
        let mut input = wire.as_slice();
        match decode_login_packet(&mut input, LoginDirection::Serverbound, TEST_MAX_FRAME)? {
            super::LoginDecodeOutcome::Complete(LoginPacket::LoginStart(_)) => {}
            _ => panic!("expected LoginStart"),
        }

        let disconnect = LoginDisconnect {
            reason: r#"{"text":"no"}"#.to_string(),
        };
        let mut wire = Vec::new();
        encode_login_disconnect(&disconnect, TEST_MAX_FRAME, &mut wire)?;
        let mut input = wire.as_slice();
        match decode_login_packet(&mut input, LoginDirection::Clientbound, TEST_MAX_FRAME)? {
            super::LoginDecodeOutcome::Complete(LoginPacket::LoginDisconnect(_)) => {}
            _ => panic!("expected LoginDisconnect"),
        }
        Ok(())
    }

    #[test]
    fn ensure_login_state_accepts_login() {
        assert!(ensure_login_state(ProtocolState::Login).is_ok());
    }

    #[test]
    fn ensure_login_state_rejects_other_states() {
        for state in [
            ProtocolState::Handshaking,
            ProtocolState::Status,
            ProtocolState::Play,
            ProtocolState::Closed,
        ] {
            assert!(ensure_login_state(state).is_err());
        }
    }

    fn sample_public_key() -> Vec<u8> {
        (0..128).map(|i| i as u8).collect()
    }

    fn sample_verify_token() -> Vec<u8> {
        vec![0xde, 0xad, 0xbe, 0xef]
    }

    fn sample_shared_secret() -> Vec<u8> {
        vec![0x42; SHARED_SECRET_LENGTH]
    }

    #[test]
    fn encryption_request_round_trips_with_empty_server_id() -> Result<(), LoginError> {
        let packet = EncryptionRequest {
            server_id: String::new(),
            public_key: sample_public_key(),
            verify_token: sample_verify_token(),
        };
        let mut wire = Vec::new();
        encode_encryption_request(&packet, TEST_MAX_FRAME, &mut wire)?;

        let mut input = wire.as_slice();
        match decode_encryption_request(&mut input, TEST_MAX_FRAME)? {
            super::LoginDecodeOutcome::Complete(LoginPacket::EncryptionRequest(decoded)) => {
                assert_eq!(decoded, packet);
            }
            _ => panic!("expected EncryptionRequest"),
        }
        assert!(input.is_empty());
        Ok(())
    }

    #[test]
    fn encryption_request_round_trips_with_non_empty_server_id() -> Result<(), LoginError> {
        let packet = EncryptionRequest {
            server_id: "myserver".to_string(),
            public_key: sample_public_key(),
            verify_token: sample_verify_token(),
        };
        let mut wire = Vec::new();
        encode_encryption_request(&packet, TEST_MAX_FRAME, &mut wire)?;

        let mut input = wire.as_slice();
        match decode_encryption_request(&mut input, TEST_MAX_FRAME)? {
            super::LoginDecodeOutcome::Complete(LoginPacket::EncryptionRequest(decoded)) => {
                assert_eq!(decoded, packet);
            }
            _ => panic!("expected EncryptionRequest"),
        }
        Ok(())
    }

    #[test]
    fn encryption_request_oversized_public_key_is_rejected() -> Result<(), LoginError> {
        let packet = EncryptionRequest {
            server_id: String::new(),
            public_key: vec![0xff; MAX_PUBLIC_KEY_LENGTH + 1],
            verify_token: sample_verify_token(),
        };
        let result = encode_encryption_request(&packet, TEST_MAX_FRAME, &mut Vec::new());
        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn encryption_request_oversized_verify_token_is_rejected() -> Result<(), LoginError> {
        let packet = EncryptionRequest {
            server_id: String::new(),
            public_key: sample_public_key(),
            verify_token: vec![0xff; MAX_VERIFY_TOKEN_LENGTH + 1],
        };
        let result = encode_encryption_request(&packet, TEST_MAX_FRAME, &mut Vec::new());
        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn encryption_request_wrong_packet_id_is_rejected() -> Result<(), LoginError> {
        let mut body = Vec::new();
        crate::primitives::encode_string("", MAX_SERVER_ID_LENGTH, &mut body)
            .map_err(LoginError::from)?;
        crate::primitives::encode_byte_array(
            &sample_public_key(),
            MAX_PUBLIC_KEY_LENGTH,
            &mut body,
        )
        .map_err(LoginError::from)?;
        crate::primitives::encode_byte_array(
            &sample_verify_token(),
            MAX_VERIFY_TOKEN_LENGTH,
            &mut body,
        )
        .map_err(LoginError::from)?;

        let mut wire = Vec::new();
        crate::framing::encode_frame(0x05, &body, TEST_MAX_FRAME, &mut wire)
            .map_err(LoginError::from)?;

        let mut input = wire.as_slice();
        let result = decode_encryption_request(&mut input, TEST_MAX_FRAME);
        assert!(matches!(result, Err(LoginError::WrongPacketId { .. })));
        assert_eq!(input, wire.as_slice());
        Ok(())
    }

    #[test]
    fn encryption_request_truncated_is_incomplete() -> Result<(), LoginError> {
        let packet = EncryptionRequest {
            server_id: String::new(),
            public_key: sample_public_key(),
            verify_token: sample_verify_token(),
        };
        let mut wire = Vec::new();
        encode_encryption_request(&packet, TEST_MAX_FRAME, &mut wire)?;

        for split in 0..wire.len() {
            let mut input = &wire[..split];
            assert_eq!(
                decode_encryption_request(&mut input, TEST_MAX_FRAME)?,
                super::LoginDecodeOutcome::Incomplete
            );
            assert_eq!(input, &wire[..split]);
        }
        Ok(())
    }

    #[test]
    fn encryption_request_trailing_bytes_are_rejected() -> Result<(), LoginError> {
        let mut body = Vec::new();
        crate::primitives::encode_string("", MAX_SERVER_ID_LENGTH, &mut body)
            .map_err(LoginError::from)?;
        crate::primitives::encode_byte_array(
            &sample_public_key(),
            MAX_PUBLIC_KEY_LENGTH,
            &mut body,
        )
        .map_err(LoginError::from)?;
        crate::primitives::encode_byte_array(
            &sample_verify_token(),
            MAX_VERIFY_TOKEN_LENGTH,
            &mut body,
        )
        .map_err(LoginError::from)?;
        body.push(0xff);

        let mut wire = Vec::new();
        crate::framing::encode_frame(
            ENCRYPTION_REQUEST_PACKET_ID,
            &body,
            TEST_MAX_FRAME,
            &mut wire,
        )
        .map_err(LoginError::from)?;

        let mut input = wire.as_slice();
        let result = decode_encryption_request(&mut input, TEST_MAX_FRAME);
        assert!(matches!(result, Err(LoginError::TrailingBytes { .. })));
        Ok(())
    }

    #[test]
    fn encryption_response_round_trips() -> Result<(), LoginError> {
        let packet = EncryptionResponse {
            shared_secret: sample_shared_secret(),
            verify_token: sample_verify_token(),
        };
        let mut wire = Vec::new();
        encode_encryption_response(&packet, TEST_MAX_FRAME, &mut wire)?;

        let mut input = wire.as_slice();
        match decode_encryption_response(&mut input, TEST_MAX_FRAME)? {
            super::LoginDecodeOutcome::Complete(LoginPacket::EncryptionResponse(decoded)) => {
                assert_eq!(decoded, packet);
            }
            _ => panic!("expected EncryptionResponse"),
        }
        assert!(input.is_empty());
        Ok(())
    }

    #[test]
    fn encryption_response_oversized_shared_secret_is_rejected() -> Result<(), LoginError> {
        let packet = EncryptionResponse {
            shared_secret: vec![0xff; MAX_PUBLIC_KEY_LENGTH + 1],
            verify_token: sample_verify_token(),
        };
        let result = encode_encryption_response(&packet, TEST_MAX_FRAME, &mut Vec::new());
        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn encryption_response_oversized_verify_token_is_rejected() -> Result<(), LoginError> {
        let packet = EncryptionResponse {
            shared_secret: sample_shared_secret(),
            verify_token: vec![0xff; MAX_VERIFY_TOKEN_LENGTH + 1],
        };
        let result = encode_encryption_response(&packet, TEST_MAX_FRAME, &mut Vec::new());
        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn encryption_response_wrong_packet_id_is_rejected() -> Result<(), LoginError> {
        let mut body = Vec::new();
        crate::primitives::encode_byte_array(
            &sample_shared_secret(),
            MAX_PUBLIC_KEY_LENGTH,
            &mut body,
        )
        .map_err(LoginError::from)?;
        crate::primitives::encode_byte_array(
            &sample_verify_token(),
            MAX_VERIFY_TOKEN_LENGTH,
            &mut body,
        )
        .map_err(LoginError::from)?;

        let mut wire = Vec::new();
        crate::framing::encode_frame(0x05, &body, TEST_MAX_FRAME, &mut wire)
            .map_err(LoginError::from)?;

        let mut input = wire.as_slice();
        let result = decode_encryption_response(&mut input, TEST_MAX_FRAME);
        assert!(matches!(result, Err(LoginError::WrongPacketId { .. })));
        assert_eq!(input, wire.as_slice());
        Ok(())
    }

    #[test]
    fn encryption_response_truncated_is_incomplete() -> Result<(), LoginError> {
        let packet = EncryptionResponse {
            shared_secret: sample_shared_secret(),
            verify_token: sample_verify_token(),
        };
        let mut wire = Vec::new();
        encode_encryption_response(&packet, TEST_MAX_FRAME, &mut wire)?;

        for split in 0..wire.len() {
            let mut input = &wire[..split];
            assert_eq!(
                decode_encryption_response(&mut input, TEST_MAX_FRAME)?,
                super::LoginDecodeOutcome::Incomplete
            );
            assert_eq!(input, &wire[..split]);
        }
        Ok(())
    }

    #[test]
    fn encryption_response_trailing_bytes_are_rejected() -> Result<(), LoginError> {
        let mut body = Vec::new();
        crate::primitives::encode_byte_array(
            &sample_shared_secret(),
            MAX_PUBLIC_KEY_LENGTH,
            &mut body,
        )
        .map_err(LoginError::from)?;
        crate::primitives::encode_byte_array(
            &sample_verify_token(),
            MAX_VERIFY_TOKEN_LENGTH,
            &mut body,
        )
        .map_err(LoginError::from)?;
        body.push(0xff);

        let mut wire = Vec::new();
        crate::framing::encode_frame(
            ENCRYPTION_RESPONSE_PACKET_ID,
            &body,
            TEST_MAX_FRAME,
            &mut wire,
        )
        .map_err(LoginError::from)?;

        let mut input = wire.as_slice();
        let result = decode_encryption_response(&mut input, TEST_MAX_FRAME);
        assert!(matches!(result, Err(LoginError::TrailingBytes { .. })));
        Ok(())
    }
}
