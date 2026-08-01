//! Incremental framing for uncompressed protocol 763 packets.
//!
//! [`decode_frame`] consumes exactly one complete frame. Callers can loop while
//! it returns [`DecodeOutcome::Complete`] to process coalesced frames. On
//! [`DecodeOutcome::Incomplete`], the input is unchanged so the caller can
//! retain it, append more bytes, and retry.

use std::fmt;

use crate::primitives::{CodecError, decode_var_int, encode_var_int};

/// Absolute Minecraft protocol limit for an uncompressed packet frame body.
///
/// The frame-length prefix is a VarInt that must fit in at most three bytes,
/// so the largest encodable body length is `2^21 - 1`.
pub const PROTOCOL_MAX_FRAME_LENGTH: usize = 0x001f_ffff;

/// Maximum number of bytes allowed for the frame-length prefix VarInt.
const MAX_LENGTH_PREFIX_BYTES: usize = 3;

/// A decoded packet borrowing its payload from the input buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PacketFrame<'a> {
    /// The packet identifier at the start of the frame body.
    pub packet_id: i32,
    /// The bytes after the packet identifier.
    pub payload: &'a [u8],
}

/// The result of attempting to decode one frame from incremental input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeOutcome<'a> {
    /// One complete frame was consumed.
    Complete(PacketFrame<'a>),
    /// More bytes are required and no input was consumed.
    Incomplete,
}

/// The result of attempting to decode one raw frame body from incremental input.
///
/// Unlike [`DecodeOutcome`], this does not extract a packet ID and returns the
/// raw body bytes directly. It is used by the compression layer where the frame
/// body has a different internal structure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawDecodeOutcome<'a> {
    /// One complete raw frame body was consumed.
    Complete(&'a [u8]),
    /// More bytes are required and no input was consumed.
    Incomplete,
}

/// An error encountered while encoding or decoding a packet frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FramingError {
    /// The frame-length VarInt is malformed.
    LengthCodec(CodecError),
    /// The frame-length prefix uses more than three encoded bytes.
    LengthPrefixTooLong { length: usize },
    /// The packet-ID VarInt is malformed or incomplete within a complete frame.
    PacketIdCodec(CodecError),
    /// The declared frame length is negative.
    NegativeFrameLength,
    /// A frame body cannot be empty because it must contain a packet ID.
    ZeroFrameLength,
    /// The frame exceeds the effective maximum.
    FrameTooLong { length: usize, maximum: usize },
    /// The packet ID and payload length cannot be represented safely.
    FrameLengthOverflow,
}

impl fmt::Display for FramingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LengthCodec(error) => write!(formatter, "invalid frame-length VarInt: {error}"),
            Self::LengthPrefixTooLong { length } => {
                write!(
                    formatter,
                    "frame-length prefix uses {length} bytes, at most 3 allowed"
                )
            }
            Self::PacketIdCodec(error) => write!(formatter, "invalid packet-ID VarInt: {error}"),
            Self::NegativeFrameLength => formatter.write_str("frame length is negative"),
            Self::ZeroFrameLength => formatter.write_str("frame length is zero"),
            Self::FrameTooLong { length, maximum } => {
                write!(formatter, "frame length {length} exceeds maximum {maximum}")
            }
            Self::FrameLengthOverflow => {
                formatter.write_str("frame length cannot be represented safely")
            }
        }
    }
}

impl std::error::Error for FramingError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::LengthCodec(error) | Self::PacketIdCodec(error) => Some(error),
            Self::LengthPrefixTooLong { .. }
            | Self::NegativeFrameLength
            | Self::ZeroFrameLength
            | Self::FrameTooLong { .. }
            | Self::FrameLengthOverflow => None,
        }
    }
}

/// Returns the smaller of the caller-provided maximum and the protocol maximum.
fn effective_max_length(caller_maximum: usize) -> usize {
    caller_maximum.min(PROTOCOL_MAX_FRAME_LENGTH)
}

/// Decodes zero or one uncompressed packet frame transactionally.
///
/// The frame-length prefix is bounded to three encoded bytes and the body is
/// bounded by the smaller of `max_frame_length` and
/// [`PROTOCOL_MAX_FRAME_LENGTH`]. A complete result advances `input` past
/// exactly one frame, preserving any following bytes. Incomplete and error
/// results leave `input` unchanged.
pub fn decode_frame<'a>(
    input: &mut &'a [u8],
    max_frame_length: usize,
) -> Result<DecodeOutcome<'a>, FramingError> {
    let source = *input;
    let mut body_and_tail = source;
    let encoded_length = match decode_var_int(&mut body_and_tail) {
        Ok(length) => length,
        Err(CodecError::IncompleteInput) => return Ok(DecodeOutcome::Incomplete),
        Err(error) => return Err(FramingError::LengthCodec(error)),
    };
    let prefix_bytes = source.len() - body_and_tail.len();
    if prefix_bytes > MAX_LENGTH_PREFIX_BYTES {
        return Err(FramingError::LengthPrefixTooLong {
            length: prefix_bytes,
        });
    }

    let frame_length =
        usize::try_from(encoded_length).map_err(|_| FramingError::NegativeFrameLength)?;

    if frame_length == 0 {
        return Err(FramingError::ZeroFrameLength);
    }
    let maximum = effective_max_length(max_frame_length);
    if frame_length > maximum {
        return Err(FramingError::FrameTooLong {
            length: frame_length,
            maximum,
        });
    }

    let Some(body) = body_and_tail.get(..frame_length) else {
        return Ok(DecodeOutcome::Incomplete);
    };
    let mut payload = body;
    let packet_id = decode_var_int(&mut payload).map_err(FramingError::PacketIdCodec)?;

    *input = &body_and_tail[frame_length..];
    Ok(DecodeOutcome::Complete(PacketFrame { packet_id, payload }))
}

/// Decodes one raw frame body (without extracting a packet ID) transactionally.
///
/// This is used by the compression layer where the frame body has a different
/// internal structure (`[Data Length] [compressed data]` rather than
/// `[Packet ID] [data]`). Returns the raw body bytes on success.
pub fn decode_raw_frame<'a>(
    input: &mut &'a [u8],
    max_frame_length: usize,
) -> Result<RawDecodeOutcome<'a>, FramingError> {
    let source = *input;
    let mut body_and_tail = source;
    let encoded_length = match decode_var_int(&mut body_and_tail) {
        Ok(length) => length,
        Err(CodecError::IncompleteInput) => return Ok(RawDecodeOutcome::Incomplete),
        Err(error) => return Err(FramingError::LengthCodec(error)),
    };
    let prefix_bytes = source.len() - body_and_tail.len();
    if prefix_bytes > MAX_LENGTH_PREFIX_BYTES {
        return Err(FramingError::LengthPrefixTooLong {
            length: prefix_bytes,
        });
    }

    let frame_length =
        usize::try_from(encoded_length).map_err(|_| FramingError::NegativeFrameLength)?;

    if frame_length == 0 {
        return Err(FramingError::ZeroFrameLength);
    }
    let maximum = effective_max_length(max_frame_length);
    if frame_length > maximum {
        return Err(FramingError::FrameTooLong {
            length: frame_length,
            maximum,
        });
    }

    let Some(body) = body_and_tail.get(..frame_length) else {
        return Ok(RawDecodeOutcome::Incomplete);
    };

    *input = &body_and_tail[frame_length..];
    Ok(RawDecodeOutcome::Complete(body))
}

/// Appends one raw frame (length prefix + body, no packet ID) after validation.
///
/// This is used by the compression layer. On error, `output` is unchanged.
pub fn encode_raw_frame(
    body: &[u8],
    max_frame_length: usize,
    output: &mut Vec<u8>,
) -> Result<(), FramingError> {
    let maximum = effective_max_length(max_frame_length);
    if body.is_empty() {
        return Err(FramingError::ZeroFrameLength);
    }
    let frame_length = body.len();
    if frame_length > maximum {
        return Err(FramingError::FrameTooLong {
            length: frame_length,
            maximum,
        });
    }
    let frame_length_i32 =
        i32::try_from(frame_length).map_err(|_| FramingError::FrameLengthOverflow)?;

    encode_var_int(frame_length_i32, output);
    output.extend_from_slice(body);
    Ok(())
}

/// Appends one uncompressed packet frame after validating its complete length.
///
/// `max_frame_length` applies to the packet ID plus payload, excluding the
/// frame-length prefix, and is clamped to [`PROTOCOL_MAX_FRAME_LENGTH`]. On
/// error, `output` is unchanged.
pub fn encode_frame(
    packet_id: i32,
    payload: &[u8],
    max_frame_length: usize,
    output: &mut Vec<u8>,
) -> Result<(), FramingError> {
    let encoded_length = validated_frame_length(packet_id, payload.len(), max_frame_length)?;

    encode_var_int(encoded_length, output);
    encode_var_int(packet_id, output);
    output.extend_from_slice(payload);
    Ok(())
}

fn validated_frame_length(
    packet_id: i32,
    payload_length: usize,
    max_frame_length: usize,
) -> Result<i32, FramingError> {
    let maximum = effective_max_length(max_frame_length);
    let frame_length = var_int_length(packet_id)
        .checked_add(payload_length)
        .ok_or(FramingError::FrameLengthOverflow)?;
    if frame_length > maximum {
        return Err(FramingError::FrameTooLong {
            length: frame_length,
            maximum,
        });
    }
    i32::try_from(frame_length).map_err(|_| FramingError::FrameLengthOverflow)
}

fn var_int_length(value: i32) -> usize {
    let bits = value as u32;
    if bits & !0x7f == 0 {
        1
    } else if bits & !0x3fff == 0 {
        2
    } else if bits & !0x1f_ffff == 0 {
        3
    } else if bits & !0x0fff_ffff == 0 {
        4
    } else {
        5
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DecodeOutcome, FramingError, PROTOCOL_MAX_FRAME_LENGTH, PacketFrame, decode_frame,
        encode_frame, validated_frame_length,
    };
    use crate::primitives::CodecError;

    #[test]
    fn empty_payload_and_multibyte_packet_id_encode_correctly() -> Result<(), FramingError> {
        let mut empty_payload = Vec::new();
        encode_frame(0, &[], 1, &mut empty_payload)?;
        assert_eq!(empty_payload, [0x01, 0x00]);

        let mut multibyte_id = Vec::new();
        encode_frame(300, &[0xaa], 3, &mut multibyte_id)?;
        assert_eq!(multibyte_id, [0x03, 0xac, 0x02, 0xaa]);
        Ok(())
    }

    #[test]
    fn every_split_point_is_transactional() -> Result<(), FramingError> {
        let payload = [0x5a; 128];
        let mut encoded = Vec::new();
        encode_frame(300, &payload, 256, &mut encoded)?;
        assert_eq!(&encoded[..2], [0x82, 0x01]);

        for split in 0..encoded.len() {
            let available = &encoded[..split];
            let mut input = available;
            assert_eq!(decode_frame(&mut input, 256), Ok(DecodeOutcome::Incomplete));
            assert_eq!(input, available);
        }

        let mut input = encoded.as_slice();
        assert_eq!(
            decode_frame(&mut input, 256),
            Ok(DecodeOutcome::Complete(PacketFrame {
                packet_id: 300,
                payload: &payload,
            }))
        );
        assert!(input.is_empty());
        Ok(())
    }

    #[test]
    fn split_at_packet_id_boundary_is_transactional() -> Result<(), FramingError> {
        let mut encoded = Vec::new();
        // packet_id 300 encodes as two bytes: 0xac 0x02.
        encode_frame(300, &[0xaa], 64, &mut encoded)?;
        // encoded = [0x03, 0xac, 0x02, 0xaa]

        // Split after the length prefix but before the packet ID.
        let mut input = &encoded[..1];
        assert_eq!(decode_frame(&mut input, 64), Ok(DecodeOutcome::Incomplete));
        assert_eq!(input, &encoded[..1]);

        // Split inside the packet ID VarInt.
        let mut input = &encoded[..2];
        assert_eq!(decode_frame(&mut input, 64), Ok(DecodeOutcome::Incomplete));
        assert_eq!(input, &encoded[..2]);
        Ok(())
    }

    #[test]
    fn coalesced_frames_decode_sequentially_and_preserve_trailing_bytes() -> Result<(), FramingError>
    {
        let mut encoded = Vec::new();
        encode_frame(1, &[0xaa], 64, &mut encoded)?;
        encode_frame(128, &[0xbb, 0xcc], 64, &mut encoded)?;
        encoded.extend_from_slice(&[0x80]);

        let mut input = encoded.as_slice();
        assert_eq!(
            decode_frame(&mut input, 64),
            Ok(DecodeOutcome::Complete(PacketFrame {
                packet_id: 1,
                payload: &[0xaa],
            }))
        );
        assert_eq!(
            decode_frame(&mut input, 64),
            Ok(DecodeOutcome::Complete(PacketFrame {
                packet_id: 128,
                payload: &[0xbb, 0xcc],
            }))
        );
        let trailing = input;
        assert_eq!(decode_frame(&mut input, 64), Ok(DecodeOutcome::Incomplete));
        assert_eq!(input, trailing);
        assert_eq!(input, [0x80]);
        Ok(())
    }

    #[test]
    fn incomplete_length_and_body_preserve_all_input() {
        for encoded in [&[][..], &[0x80][..], &[0x03, 0x01, 0xaa][..]] {
            let mut input = encoded;
            assert_eq!(decode_frame(&mut input, 64), Ok(DecodeOutcome::Incomplete));
            assert_eq!(input, encoded);
        }
    }

    #[test]
    fn rejects_zero_and_oversized_lengths_transactionally() {
        // Note: negative frame lengths require a five-byte prefix, which is
        // rejected by LengthPrefixTooLong before the value is inspected. The
        // dedicated prefix-length tests cover that path.
        let cases = [
            (&[0x00][..], FramingError::ZeroFrameLength),
            (
                &[0x03][..],
                FramingError::FrameTooLong {
                    length: 3,
                    maximum: 2,
                },
            ),
        ];

        for (encoded, expected) in cases {
            let mut input = encoded;
            assert_eq!(decode_frame(&mut input, 2), Err(expected));
            assert_eq!(input, encoded);
        }
    }

    #[test]
    fn distinguishes_malformed_length_and_packet_id_var_ints() {
        let malformed_length = &[0x80, 0x80, 0x80, 0x80, 0x80][..];
        let mut input = malformed_length;
        assert_eq!(
            decode_frame(&mut input, 64),
            Err(FramingError::LengthCodec(CodecError::VarIntTooLong))
        );
        assert_eq!(input, malformed_length);

        let malformed_packet_id = &[0x05, 0x80, 0x80, 0x80, 0x80, 0x80][..];
        let mut input = malformed_packet_id;
        assert_eq!(
            decode_frame(&mut input, 64),
            Err(FramingError::PacketIdCodec(CodecError::VarIntTooLong))
        );
        assert_eq!(input, malformed_packet_id);
    }

    #[test]
    fn declared_length_shorter_than_packet_id_is_a_framing_error() {
        let encoded = &[0x01, 0x80][..];
        let mut input = encoded;
        assert_eq!(
            decode_frame(&mut input, 64),
            Err(FramingError::PacketIdCodec(CodecError::IncompleteInput))
        );
        assert_eq!(input, encoded);
    }

    #[test]
    fn encode_rejects_limits_without_mutating_output() {
        let mut output = vec![0xaa];
        assert_eq!(
            encode_frame(128, &[0x01], 2, &mut output),
            Err(FramingError::FrameTooLong {
                length: 3,
                maximum: 2,
            })
        );
        assert_eq!(output, [0xaa]);
    }

    #[test]
    fn encode_rejects_addition_overflow() {
        // With the protocol maximum clamped to 2^21-1, payloads above that
        // always hit FrameTooLong first. Only checked_add overflow reaches
        // FrameLengthOverflow.
        assert_eq!(
            validated_frame_length(0, usize::MAX, usize::MAX),
            Err(FramingError::FrameLengthOverflow)
        );
    }

    #[test]
    fn representative_frames_round_trip() -> Result<(), FramingError> {
        let cases = [
            (0, &[][..]),
            (1, &[0x00][..]),
            (127, &[0x01, 0x02][..]),
            (128, &[0xff][..]),
            (i32::MAX, &[0x10, 0x20, 0x30][..]),
            (-1, &[0x7f][..]),
        ];

        for (packet_id, payload) in cases {
            let mut encoded = Vec::new();
            encode_frame(packet_id, payload, 64, &mut encoded)?;
            let mut input = encoded.as_slice();
            assert_eq!(
                decode_frame(&mut input, 64),
                Ok(DecodeOutcome::Complete(PacketFrame { packet_id, payload }))
            );
            assert!(input.is_empty());
        }
        Ok(())
    }

    #[test]
    fn rejects_non_minimal_four_byte_length_prefix_transactionally() {
        // 0x80 0x80 0x80 0x00 is a four-byte non-minimal encoding of 0.
        let encoded = &[0x80, 0x80, 0x80, 0x00, 0x01, 0x00][..];
        let mut input = encoded;
        assert_eq!(
            decode_frame(&mut input, 64),
            Err(FramingError::LengthPrefixTooLong { length: 4 })
        );
        assert_eq!(input, encoded);
    }

    #[test]
    fn rejects_non_minimal_five_byte_length_prefix_transactionally() {
        // 0x80 0x80 0x80 0x80 0x00 is a five-byte non-minimal encoding of 0.
        let encoded = &[0x80, 0x80, 0x80, 0x80, 0x00, 0x01, 0x00][..];
        let mut input = encoded;
        assert_eq!(
            decode_frame(&mut input, 64),
            Err(FramingError::LengthPrefixTooLong { length: 5 })
        );
        assert_eq!(input, encoded);
    }

    #[test]
    fn accepts_three_byte_length_prefix_at_protocol_boundary() -> Result<(), FramingError> {
        // The largest three-byte VarInt is 0xff 0xff 0x7f = 2,097,151.
        // A frame body of that size begins right after the prefix. We only
        // verify the prefix is accepted and the body is reported incomplete
        // without allocating a multi-megabyte buffer.
        let prefix_and_partial = &[0xff, 0xff, 0x7f, 0x00][..];
        let mut input = prefix_and_partial;
        assert_eq!(
            decode_frame(&mut input, usize::MAX),
            Ok(DecodeOutcome::Incomplete)
        );
        assert_eq!(input, prefix_and_partial);
        Ok(())
    }

    #[test]
    fn protocol_absolute_limit_is_enforced_even_with_huge_caller_maximum() {
        // 0x80 0x80 0x80 0x01 is a four-byte prefix encoding 2,097,152. The
        // three-byte prefix limit rejects it before the value is checked.
        let encoded = &[0x80, 0x80, 0x80, 0x01][..];
        let mut input = encoded;
        assert_eq!(
            decode_frame(&mut input, usize::MAX),
            Err(FramingError::LengthPrefixTooLong { length: 4 })
        );
        assert_eq!(input, encoded);
    }

    #[test]
    fn encode_clamps_to_protocol_absolute_limit() {
        // A caller maximum above the protocol maximum must still be clamped.
        // var_int_length(0) == 1, so frame_length == 1 + payload_length.
        assert_eq!(
            validated_frame_length(0, PROTOCOL_MAX_FRAME_LENGTH, usize::MAX),
            Err(FramingError::FrameTooLong {
                length: PROTOCOL_MAX_FRAME_LENGTH + 1,
                maximum: PROTOCOL_MAX_FRAME_LENGTH,
            })
        );
    }

    #[test]
    fn accepts_non_minimal_packet_id_varint() -> Result<(), FramingError> {
        // Frame: length=3, packet_id encoded non-minimally as 0x81 0x00 (=1),
        // payload=[0xaa].
        let encoded = &[0x03, 0x81, 0x00, 0xaa][..];
        let mut input = encoded;
        assert_eq!(
            decode_frame(&mut input, 64),
            Ok(DecodeOutcome::Complete(PacketFrame {
                packet_id: 1,
                payload: &[0xaa],
            }))
        );
        assert!(input.is_empty());
        Ok(())
    }
}
