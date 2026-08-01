//! Compression layer for protocol 763 packets.
//!
//! After a Set Compression packet with a non-negative threshold is sent, all
//! subsequent packets use the compressed frame format:
//!
//! ```text
//! [Packet Length VarInt] [Data Length VarInt] [compressed: Packet ID + data]
//! ```
//!
//! When the uncompressed payload size is less than the threshold, the packet
//! may be sent uncompressed, in which case the Data Length is `0` and the
//! Packet ID + data follow directly without zlib compression.
//!
//! The Set Compression packet itself is sent uncompressed.

use std::fmt;
use std::io::{Read, Write};

use crate::framing::{
    DecodeOutcome, FramingError, PROTOCOL_MAX_FRAME_LENGTH, RawDecodeOutcome, decode_frame,
    decode_raw_frame, encode_frame, encode_raw_frame,
};
use crate::primitives::{CodecError, decode_var_int, encode_var_int};

/// Packet ID for the clientbound Set Compression packet (Login state).
pub const SET_COMPRESSION_PACKET_ID: i32 = 0x03;

/// Compression threshold value that disables compression.
pub const COMPRESSION_DISABLED: i32 = -1;

/// Maximum size of a compressed payload after decompression.
///
/// This matches the protocol's absolute frame body limit of `2^21 - 1`.
const MAX_DECOMPRESSED_SIZE: usize = PROTOCOL_MAX_FRAME_LENGTH;

/// An error encountered while encoding or decoding compressed packets.
#[derive(Debug)]
pub enum CompressionError {
    /// A framing error occurred.
    Framing(FramingError),
    /// A primitive codec error occurred.
    Codec(CodecError),
    /// The zlib stream is malformed.
    Zlib(std::io::Error),
    /// The decompressed payload exceeds the maximum allowed size.
    DecompressedTooLarge { size: usize, maximum: usize },
    /// The declared data length does not match the decompressed size.
    DataLengthMismatch { declared: usize, actual: usize },
    /// The packet ID does not match the expected value.
    WrongPacketId { received: i32, expected: i32 },
    /// The packet contained trailing bytes after the expected fields.
    TrailingBytes { count: usize },
    /// More input is required.
    Incomplete,
}

impl fmt::Display for CompressionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Framing(error) => write!(formatter, "framing error: {error}"),
            Self::Codec(error) => write!(formatter, "codec error: {error}"),
            Self::Zlib(error) => write!(formatter, "zlib error: {error}"),
            Self::DecompressedTooLarge { size, maximum } => {
                write!(
                    formatter,
                    "decompressed payload {size} exceeds maximum {maximum}"
                )
            }
            Self::DataLengthMismatch { declared, actual } => {
                write!(
                    formatter,
                    "data length {declared} does not match actual {actual}"
                )
            }
            Self::WrongPacketId { received, expected } => {
                write!(formatter, "expected packet ID {expected}, got {received}")
            }
            Self::TrailingBytes { count } => {
                write!(formatter, "packet has {count} trailing byte(s)")
            }
            Self::Incomplete => formatter.write_str("incomplete input"),
        }
    }
}

impl std::error::Error for CompressionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Framing(error) => Some(error),
            Self::Codec(error) => Some(error),
            Self::Zlib(error) => Some(error),
            Self::WrongPacketId { .. } | Self::TrailingBytes { .. } | Self::Incomplete => None,
            Self::DecompressedTooLarge { .. } | Self::DataLengthMismatch { .. } => None,
        }
    }
}

impl From<FramingError> for CompressionError {
    fn from(error: FramingError) -> Self {
        match error {
            FramingError::LengthCodec(CodecError::IncompleteInput) => Self::Incomplete,
            other => Self::Framing(other),
        }
    }
}

impl From<CodecError> for CompressionError {
    fn from(error: CodecError) -> Self {
        match error {
            CodecError::IncompleteInput => Self::Incomplete,
            other => Self::Codec(other),
        }
    }
}

/// The result of attempting to decode one compressed frame.
#[derive(Debug, PartialEq, Eq)]
pub enum CompressedDecodeOutcome {
    /// One complete decompressed frame was consumed.
    Complete(DecompressedPacket),
    /// More bytes are required and no input was consumed.
    Incomplete,
}

/// A decompressed packet with its packet ID and payload as owned bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecompressedPacket {
    /// The packet identifier.
    pub packet_id: i32,
    /// The decompressed payload bytes (after the packet ID).
    pub payload: Vec<u8>,
}

/// Decodes one compressed packet frame from incremental input.
///
/// `threshold` is the compression threshold from the most recent Set
/// Compression packet. A value of `>= 0` means compression is enabled; `-1`
/// means it is disabled (in which case this function behaves like
/// [`crate::framing::decode_frame`] but returns owned data).
///
/// On [`CompressedDecodeOutcome::Incomplete`], the input is left unchanged.
pub fn decode_compressed_frame(
    input: &mut &[u8],
    threshold: i32,
    max_frame_length: usize,
) -> Result<CompressedDecodeOutcome, CompressionError> {
    let source = *input;

    if threshold < 0 {
        // Compression disabled: standard uncompressed frame format.
        let frame = match decode_frame(input, max_frame_length) {
            Ok(DecodeOutcome::Complete(frame)) => frame,
            Ok(DecodeOutcome::Incomplete) => {
                *input = source;
                return Ok(CompressedDecodeOutcome::Incomplete);
            }
            Err(error) => {
                *input = source;
                return Err(CompressionError::from(error));
            }
        };
        return Ok(CompressedDecodeOutcome::Complete(DecompressedPacket {
            packet_id: frame.packet_id,
            payload: frame.payload.to_vec(),
        }));
    }

    // Compression enabled: decode the raw frame body (without extracting
    // a packet ID, since the body starts with a Data Length VarInt).
    let raw_body = match decode_raw_frame(input, max_frame_length) {
        Ok(RawDecodeOutcome::Complete(body)) => body,
        Ok(RawDecodeOutcome::Incomplete) => {
            *input = source;
            return Ok(CompressedDecodeOutcome::Incomplete);
        }
        Err(error) => {
            *input = source;
            return Err(CompressionError::from(error));
        }
    };

    // Parse the body: [Data Length VarInt] [compressed or uncompressed data]
    let mut body = raw_body;
    let data_length = decode_var_int(&mut body).map_err(|error| {
        *input = source;
        CompressionError::from(error)
    })?;
    let data_length = usize::try_from(data_length).map_err(|_| {
        *input = source;
        CompressionError::Codec(CodecError::VarIntTooLong)
    })?;

    if data_length == 0 {
        // Uncompressed-below-threshold: body is packet ID + data directly.
        let packet_id = decode_var_int(&mut body).map_err(|error| {
            *input = source;
            CompressionError::from(error)
        })?;
        Ok(CompressedDecodeOutcome::Complete(DecompressedPacket {
            packet_id,
            payload: body.to_vec(),
        }))
    } else {
        // Compressed: body is zlib-compressed packet ID + data.
        if data_length > MAX_DECOMPRESSED_SIZE {
            *input = source;
            return Err(CompressionError::DecompressedTooLarge {
                size: data_length,
                maximum: MAX_DECOMPRESSED_SIZE,
            });
        }

        let mut decoder = flate2::read::ZlibDecoder::new(body);
        let mut decompressed = Vec::with_capacity(data_length);
        if let Err(error) = decoder.read_to_end(&mut decompressed) {
            *input = source;
            return Err(CompressionError::Zlib(error));
        }

        if decompressed.len() != data_length {
            *input = source;
            return Err(CompressionError::DataLengthMismatch {
                declared: data_length,
                actual: decompressed.len(),
            });
        }

        // Parse packet ID from the decompressed data.
        let mut payload_input: &[u8] = &decompressed;
        let packet_id = decode_var_int(&mut payload_input).map_err(|error| {
            *input = source;
            CompressionError::from(error)
        })?;

        Ok(CompressedDecodeOutcome::Complete(DecompressedPacket {
            packet_id,
            payload: payload_input.to_vec(),
        }))
    }
}

/// Encodes one compressed packet frame into the output buffer.
///
/// If `threshold >= 0` and the uncompressed body size (packet ID + payload) is
/// greater than `threshold`, the body is zlib-compressed. Otherwise, the packet
/// is sent uncompressed with a Data Length of `0`.
///
/// If `threshold < 0`, compression is disabled and the frame is encoded as a
/// plain uncompressed frame.
///
/// On error, `output` is unchanged.
pub fn encode_compressed_frame(
    packet_id: i32,
    payload: &[u8],
    threshold: i32,
    max_frame_length: usize,
    output: &mut Vec<u8>,
) -> Result<(), CompressionError> {
    // Build the uncompressed body: packet ID + payload.
    let mut uncompressed_body = Vec::new();
    encode_var_int(packet_id, &mut uncompressed_body);
    uncompressed_body.extend_from_slice(payload);

    if threshold < 0 {
        // Compression disabled: plain uncompressed frame.
        encode_frame(packet_id, payload, max_frame_length, output)
            .map_err(CompressionError::from)?;
        return Ok(());
    }

    let uncompressed_size = uncompressed_body.len();
    let threshold = usize::try_from(threshold).unwrap_or(0);

    let mut body = Vec::new();
    if uncompressed_size > threshold {
        // Compress with zlib.
        let mut compressed = Vec::new();
        {
            let mut encoder =
                flate2::write::ZlibEncoder::new(&mut compressed, flate2::Compression::default());
            encoder
                .write_all(&uncompressed_body)
                .map_err(CompressionError::Zlib)?;
            encoder.finish().map_err(CompressionError::Zlib)?;
        }

        // Data Length = uncompressed size of (packet ID + payload).
        encode_var_int(uncompressed_size as i32, &mut body);
        body.extend_from_slice(&compressed);
    } else {
        // Below threshold: send uncompressed with Data Length = 0.
        encode_var_int(0, &mut body);
        body.extend_from_slice(&uncompressed_body);
    }

    // Wrap the body in a raw frame (length prefix + body, no packet ID).
    encode_raw_frame(&body, max_frame_length, output).map_err(CompressionError::from)?;
    Ok(())
}

/// Encodes the Set Compression packet (clientbound Login `0x03`).
///
/// This packet is always sent uncompressed. A threshold of `-1` disables
/// compression; `>= 0` enables it with the given threshold.
pub fn encode_set_compression(
    threshold: i32,
    max_frame_length: usize,
    output: &mut Vec<u8>,
) -> Result<(), CompressionError> {
    let mut body = Vec::new();
    encode_var_int(threshold, &mut body);

    encode_frame(SET_COMPRESSION_PACKET_ID, &body, max_frame_length, output)
        .map_err(CompressionError::from)?;
    Ok(())
}

/// Decodes the Set Compression packet (clientbound Login `0x03`).
///
/// Returns the threshold value. On incomplete input, the input is unchanged.
pub fn decode_set_compression(
    input: &mut &[u8],
    max_frame_length: usize,
) -> Result<Option<i32>, CompressionError> {
    let source = *input;
    let frame = match decode_frame(input, max_frame_length) {
        Ok(DecodeOutcome::Complete(frame)) => frame,
        Ok(DecodeOutcome::Incomplete) => {
            *input = source;
            return Ok(None);
        }
        Err(error) => {
            *input = source;
            return Err(CompressionError::from(error));
        }
    };

    if frame.packet_id != SET_COMPRESSION_PACKET_ID {
        *input = source;
        return Err(CompressionError::WrongPacketId {
            received: frame.packet_id,
            expected: SET_COMPRESSION_PACKET_ID,
        });
    }

    let mut body = frame.payload;
    let threshold = decode_var_int(&mut body).map_err(|error| {
        *input = source;
        CompressionError::from(error)
    })?;

    if !body.is_empty() {
        *input = source;
        return Err(CompressionError::TrailingBytes { count: body.len() });
    }

    Ok(Some(threshold))
}

#[cfg(test)]
mod tests {
    use super::{
        COMPRESSION_DISABLED, CompressionError, DecompressedPacket, SET_COMPRESSION_PACKET_ID,
        decode_compressed_frame, decode_set_compression, encode_compressed_frame,
        encode_set_compression,
    };
    use crate::framing::PROTOCOL_MAX_FRAME_LENGTH;
    use std::io::Write;

    const TEST_MAX_FRAME: usize = 65536;

    fn encode_uncompressed(
        packet_id: i32,
        payload: &[u8],
        output: &mut Vec<u8>,
    ) -> Result<(), CompressionError> {
        encode_compressed_frame(
            packet_id,
            payload,
            COMPRESSION_DISABLED,
            TEST_MAX_FRAME,
            output,
        )
    }

    #[test]
    fn set_compression_round_trips() -> Result<(), CompressionError> {
        for threshold in [-1i32, 0, 256, 1024] {
            let mut wire = Vec::new();
            encode_set_compression(threshold, TEST_MAX_FRAME, &mut wire)?;
            let mut input = wire.as_slice();
            assert_eq!(
                decode_set_compression(&mut input, TEST_MAX_FRAME)?,
                Some(threshold)
            );
            assert!(input.is_empty());
        }
        Ok(())
    }

    #[test]
    fn compression_disabled_round_trips() -> Result<(), CompressionError> {
        let payload = [0x01, 0x02, 0x03, 0x04, 0x05];
        let mut wire = Vec::new();
        encode_uncompressed(0x10, &payload, &mut wire)?;

        let mut input = wire.as_slice();
        match decode_compressed_frame(&mut input, COMPRESSION_DISABLED, TEST_MAX_FRAME)? {
            super::CompressedDecodeOutcome::Complete(packet) => {
                assert_eq!(packet.packet_id, 0x10);
                assert_eq!(packet.payload, payload);
            }
            super::CompressedDecodeOutcome::Incomplete => {
                panic!("expected complete frame");
            }
        }
        assert!(input.is_empty());
        Ok(())
    }

    #[test]
    fn compressed_above_threshold_round_trips() -> Result<(), CompressionError> {
        let payload = [0xaa; 512];
        let threshold = 256;
        let mut wire = Vec::new();
        encode_compressed_frame(0x20, &payload, threshold, TEST_MAX_FRAME, &mut wire)?;

        let mut input = wire.as_slice();
        match decode_compressed_frame(&mut input, threshold, TEST_MAX_FRAME)? {
            super::CompressedDecodeOutcome::Complete(packet) => {
                assert_eq!(packet.packet_id, 0x20);
                assert_eq!(packet.payload, payload.to_vec());
            }
            super::CompressedDecodeOutcome::Incomplete => {
                panic!("expected complete frame");
            }
        }
        assert!(input.is_empty());
        Ok(())
    }

    #[test]
    fn uncompressed_below_threshold_round_trips() -> Result<(), CompressionError> {
        let payload = [0x01, 0x02, 0x03];
        let threshold = 256;
        let mut wire = Vec::new();
        encode_compressed_frame(0x30, &payload, threshold, TEST_MAX_FRAME, &mut wire)?;

        let mut input = wire.as_slice();
        match decode_compressed_frame(&mut input, threshold, TEST_MAX_FRAME)? {
            super::CompressedDecodeOutcome::Complete(packet) => {
                assert_eq!(packet.packet_id, 0x30);
                assert_eq!(packet.payload, payload.to_vec());
            }
            super::CompressedDecodeOutcome::Incomplete => {
                panic!("expected complete frame");
            }
        }
        assert!(input.is_empty());
        Ok(())
    }

    #[test]
    fn threshold_zero_compresses_everything() -> Result<(), CompressionError> {
        let payload = [0x42];
        let threshold = 0;
        let mut wire = Vec::new();
        encode_compressed_frame(0x40, &payload, threshold, TEST_MAX_FRAME, &mut wire)?;

        let mut input = wire.as_slice();
        match decode_compressed_frame(&mut input, threshold, TEST_MAX_FRAME)? {
            super::CompressedDecodeOutcome::Complete(packet) => {
                assert_eq!(packet.packet_id, 0x40);
                assert_eq!(packet.payload, payload.to_vec());
            }
            super::CompressedDecodeOutcome::Incomplete => {
                panic!("expected complete frame");
            }
        }
        assert!(input.is_empty());
        Ok(())
    }

    #[test]
    fn empty_payload_round_trips_compressed() -> Result<(), CompressionError> {
        let payload: [u8; 0] = [];
        let threshold = 0;
        let mut wire = Vec::new();
        encode_compressed_frame(0x50, &payload, threshold, TEST_MAX_FRAME, &mut wire)?;

        let mut input = wire.as_slice();
        match decode_compressed_frame(&mut input, threshold, TEST_MAX_FRAME)? {
            super::CompressedDecodeOutcome::Complete(packet) => {
                assert_eq!(packet.packet_id, 0x50);
                assert!(packet.payload.is_empty());
            }
            super::CompressedDecodeOutcome::Incomplete => {
                panic!("expected complete frame");
            }
        }
        Ok(())
    }

    #[test]
    fn multibyte_packet_id_round_trips_compressed() -> Result<(), CompressionError> {
        let payload = [0xff; 128];
        let threshold = 64;
        let mut wire = Vec::new();
        encode_compressed_frame(300, &payload, threshold, TEST_MAX_FRAME, &mut wire)?;

        let mut input = wire.as_slice();
        match decode_compressed_frame(&mut input, threshold, TEST_MAX_FRAME)? {
            super::CompressedDecodeOutcome::Complete(packet) => {
                assert_eq!(packet.packet_id, 300);
                assert_eq!(packet.payload, payload.to_vec());
            }
            super::CompressedDecodeOutcome::Incomplete => {
                panic!("expected complete frame");
            }
        }
        Ok(())
    }

    #[test]
    fn fragmented_input_is_incomplete_and_preserves_buffer() -> Result<(), CompressionError> {
        let payload = [0xaa; 512];
        let threshold = 256;
        let mut wire = Vec::new();
        encode_compressed_frame(0x20, &payload, threshold, TEST_MAX_FRAME, &mut wire)?;

        for split in 0..wire.len() {
            let mut input = &wire[..split];
            assert_eq!(
                decode_compressed_frame(&mut input, threshold, TEST_MAX_FRAME)?,
                super::CompressedDecodeOutcome::Incomplete
            );
            assert_eq!(input, &wire[..split]);
        }
        Ok(())
    }

    #[test]
    fn coalesced_compressed_frames_decode_sequentially() -> Result<(), CompressionError> {
        let threshold = 64;
        let mut wire = Vec::new();
        encode_compressed_frame(1, &[0xaa], threshold, TEST_MAX_FRAME, &mut wire)?;
        encode_compressed_frame(2, &[0xbb, 0xcc], threshold, TEST_MAX_FRAME, &mut wire)?;

        let mut input = wire.as_slice();
        match decode_compressed_frame(&mut input, threshold, TEST_MAX_FRAME)? {
            super::CompressedDecodeOutcome::Complete(p) => {
                assert_eq!(
                    p,
                    DecompressedPacket {
                        packet_id: 1,
                        payload: vec![0xaa]
                    }
                );
            }
            _ => panic!("expected complete"),
        }
        match decode_compressed_frame(&mut input, threshold, TEST_MAX_FRAME)? {
            super::CompressedDecodeOutcome::Complete(p) => {
                assert_eq!(
                    p,
                    DecompressedPacket {
                        packet_id: 2,
                        payload: vec![0xbb, 0xcc]
                    }
                );
            }
            _ => panic!("expected complete"),
        }
        assert!(input.is_empty());
        Ok(())
    }

    #[test]
    fn malformed_zlib_stream_is_rejected() -> Result<(), CompressionError> {
        // Manually construct a frame with an invalid zlib stream.
        let threshold = 0;
        let mut body = Vec::new();
        // Data Length = 100 (claiming 100 bytes of decompressed data)
        crate::primitives::encode_var_int(100, &mut body);
        // Invalid zlib data
        body.extend_from_slice(&[0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]);

        let mut wire = Vec::new();
        crate::framing::encode_raw_frame(&body, TEST_MAX_FRAME, &mut wire)
            .map_err(CompressionError::from)?;

        let mut input = wire.as_slice();
        let result = decode_compressed_frame(&mut input, threshold, TEST_MAX_FRAME);
        assert!(matches!(result, Err(CompressionError::Zlib(_))));
        // Input is restored on error.
        assert_eq!(input, wire.as_slice());
        Ok(())
    }

    #[test]
    fn data_length_mismatch_is_rejected() -> Result<(), CompressionError> {
        let threshold = 0;
        // Compress 10 bytes but declare data length as 20.
        let raw = [0x42u8; 10];
        let mut compressed = Vec::new();
        {
            let mut encoder =
                flate2::write::ZlibEncoder::new(&mut compressed, flate2::Compression::default());
            encoder.write_all(&raw).map_err(CompressionError::Zlib)?;
            encoder.finish().map_err(CompressionError::Zlib)?;
        }

        let mut body = Vec::new();
        crate::primitives::encode_var_int(20, &mut body); // wrong data length
        body.extend_from_slice(&compressed);

        let mut wire = Vec::new();
        crate::framing::encode_raw_frame(&body, TEST_MAX_FRAME, &mut wire)
            .map_err(CompressionError::from)?;

        let mut input = wire.as_slice();
        let result = decode_compressed_frame(&mut input, threshold, TEST_MAX_FRAME);
        assert!(matches!(
            result,
            Err(CompressionError::DataLengthMismatch { .. })
        ));
        assert_eq!(input, wire.as_slice());
        Ok(())
    }

    #[test]
    fn set_compression_with_wrong_packet_id_is_rejected() -> Result<(), CompressionError> {
        let mut body = Vec::new();
        crate::primitives::encode_var_int(256, &mut body);
        let mut wire = Vec::new();
        crate::framing::encode_frame(0x05, &body, TEST_MAX_FRAME, &mut wire)
            .map_err(CompressionError::from)?;

        let mut input = wire.as_slice();
        let result = decode_set_compression(&mut input, TEST_MAX_FRAME);
        assert!(result.is_err());
        assert_eq!(input, wire.as_slice());
        Ok(())
    }

    #[test]
    fn set_compression_truncated_is_incomplete() -> Result<(), CompressionError> {
        let mut wire = Vec::new();
        encode_set_compression(256, TEST_MAX_FRAME, &mut wire)?;

        for split in 0..wire.len() {
            let mut input = &wire[..split];
            assert_eq!(decode_set_compression(&mut input, TEST_MAX_FRAME)?, None);
            assert_eq!(input, &wire[..split]);
        }
        Ok(())
    }

    #[test]
    fn set_compression_with_trailing_bytes_is_rejected() -> Result<(), CompressionError> {
        let mut body = Vec::new();
        crate::primitives::encode_var_int(256, &mut body);
        body.push(0xff); // trailing byte
        let mut wire = Vec::new();
        crate::framing::encode_frame(SET_COMPRESSION_PACKET_ID, &body, TEST_MAX_FRAME, &mut wire)
            .map_err(CompressionError::from)?;

        let mut input = wire.as_slice();
        let result = decode_set_compression(&mut input, TEST_MAX_FRAME);
        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn decompressed_payload_above_protocol_limit_is_rejected() -> Result<(), CompressionError> {
        let threshold = 0;
        // Declare a data length larger than the protocol maximum.
        let mut body = Vec::new();
        crate::primitives::encode_var_int((PROTOCOL_MAX_FRAME_LENGTH + 1) as i32, &mut body);
        body.extend_from_slice(&[0x78, 0x01]); // zlib header start

        let mut wire = Vec::new();
        crate::framing::encode_raw_frame(&body, TEST_MAX_FRAME, &mut wire)
            .map_err(CompressionError::from)?;

        let mut input = wire.as_slice();
        let result = decode_compressed_frame(&mut input, threshold, TEST_MAX_FRAME);
        assert!(matches!(
            result,
            Err(CompressionError::DecompressedTooLarge { .. })
        ));
        assert_eq!(input, wire.as_slice());
        Ok(())
    }
}
