//! Bounded codecs for the primitive values used by protocol 763.
//!
//! All decoders are transactional: `input` is advanced only when the complete
//! value has been decoded successfully. This allows callers to retain partial
//! input and retry once more bytes arrive.

use std::fmt;

/// An error encountered while encoding or decoding a protocol primitive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodecError {
    /// More bytes are required to decode the requested value.
    IncompleteInput,
    /// A VarInt still has its continuation bit set on its fifth byte.
    VarIntTooLong,
    /// A VarLong still has its continuation bit set on its tenth byte.
    VarLongTooLong,
    /// A String's encoded byte length is negative.
    NegativeStringLength,
    /// A String exceeds its caller-provided UTF-16 unit or derived byte limit.
    StringTooLong,
    /// A String's bytes are not valid UTF-8.
    InvalidUtf8,
    /// A boolean byte was not 0 or 1.
    InvalidBoolean,
}

impl fmt::Display for CodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::IncompleteInput => "incomplete protocol input",
            Self::VarIntTooLong => "VarInt is overlong",
            Self::VarLongTooLong => "VarLong is overlong",
            Self::NegativeStringLength => "String has a negative byte length",
            Self::StringTooLong => "String exceeds its configured limit",
            Self::InvalidUtf8 => "String is not valid UTF-8",
            Self::InvalidBoolean => "boolean byte was not 0 or 1",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for CodecError {}

/// Appends an `i32` using Minecraft's two's-complement VarInt encoding.
pub fn encode_var_int(value: i32, output: &mut Vec<u8>) {
    let mut remaining = value as u32;
    loop {
        let mut byte = (remaining & 0x7f) as u8;
        remaining >>= 7;
        if remaining != 0 {
            byte |= 0x80;
        }
        output.push(byte);
        if remaining == 0 {
            return;
        }
    }
}

/// Decodes an `i32` Minecraft VarInt transactionally.
pub fn decode_var_int(input: &mut &[u8]) -> Result<i32, CodecError> {
    let source = *input;
    let mut result = 0_u32;

    for index in 0..5 {
        let byte = *source.get(index).ok_or(CodecError::IncompleteInput)?;
        let payload = u32::from(byte & 0x7f);
        result |= payload << (index * 7);

        if byte & 0x80 == 0 {
            *input = &source[index + 1..];
            return Ok(result as i32);
        }
    }

    Err(CodecError::VarIntTooLong)
}

/// Appends an `i64` using Minecraft's two's-complement VarLong encoding.
pub fn encode_var_long(value: i64, output: &mut Vec<u8>) {
    let mut remaining = value as u64;
    loop {
        let mut byte = (remaining & 0x7f) as u8;
        remaining >>= 7;
        if remaining != 0 {
            byte |= 0x80;
        }
        output.push(byte);
        if remaining == 0 {
            return;
        }
    }
}

/// Decodes an `i64` Minecraft VarLong transactionally.
pub fn decode_var_long(input: &mut &[u8]) -> Result<i64, CodecError> {
    let source = *input;
    let mut result = 0_u64;

    for index in 0..10 {
        let byte = *source.get(index).ok_or(CodecError::IncompleteInput)?;
        let payload = u64::from(byte & 0x7f);
        result |= payload << (index * 7);

        if byte & 0x80 == 0 {
            *input = &source[index + 1..];
            return Ok(result as i64);
        }
    }

    Err(CodecError::VarLongTooLong)
}

/// Appends a UTF-8 String prefixed by its VarInt byte length.
///
/// `max_utf16_units` limits the number of UTF-16 code units, matching the
/// protocol's Java String length semantics. The encoded byte length is also
/// bounded to at most three bytes per allowed UTF-16 unit and must fit in an
/// `i32` VarInt.
pub fn encode_string(
    value: &str,
    max_utf16_units: usize,
    output: &mut Vec<u8>,
) -> Result<(), CodecError> {
    let max_bytes = max_utf16_units.saturating_mul(3);
    if value.len() > max_bytes
        || value.len() > i32::MAX as usize
        || value.encode_utf16().count() > max_utf16_units
    {
        return Err(CodecError::StringTooLong);
    }

    encode_var_int(value.len() as i32, output);
    output.extend_from_slice(value.as_bytes());
    Ok(())
}

/// Decodes a VarInt-length-prefixed UTF-8 String transactionally.
///
/// `max_utf16_units` limits the number of UTF-16 code units, matching the
/// protocol's Java String length semantics. Before accessing the payload, the
/// declared byte length is checked against the safe upper bound of three bytes
/// per allowed UTF-16 unit.
pub fn decode_string<'a>(
    input: &mut &'a [u8],
    max_utf16_units: usize,
) -> Result<&'a str, CodecError> {
    let source = *input;
    let mut remaining = source;
    let encoded_len = decode_var_int(&mut remaining)?;
    let byte_len = usize::try_from(encoded_len).map_err(|_| CodecError::NegativeStringLength)?;
    let max_bytes = max_utf16_units.saturating_mul(3);

    if byte_len > max_bytes {
        return Err(CodecError::StringTooLong);
    }

    let bytes = remaining
        .get(..byte_len)
        .ok_or(CodecError::IncompleteInput)?;
    let value = std::str::from_utf8(bytes).map_err(|_| CodecError::InvalidUtf8)?;
    if value.encode_utf16().count() > max_utf16_units {
        return Err(CodecError::StringTooLong);
    }

    *input = &remaining[byte_len..];
    Ok(value)
}

/// Appends an unsigned 16-bit integer in network (big-endian) byte order.
pub fn encode_u16(value: u16, output: &mut Vec<u8>) {
    output.extend_from_slice(&value.to_be_bytes());
}

/// Decodes an unsigned network-order 16-bit integer transactionally.
pub fn decode_u16(input: &mut &[u8]) -> Result<u16, CodecError> {
    let source = *input;
    let bytes = source.get(..2).ok_or(CodecError::IncompleteInput)?;
    let value = u16::from_be_bytes([bytes[0], bytes[1]]);
    *input = &source[2..];
    Ok(value)
}

/// Appends a signed 64-bit integer in network (big-endian) byte order.
pub fn encode_i64(value: i64, output: &mut Vec<u8>) {
    output.extend_from_slice(&value.to_be_bytes());
}

/// Decodes a signed network-order 64-bit integer transactionally.
pub fn decode_i64(input: &mut &[u8]) -> Result<i64, CodecError> {
    let source = *input;
    let bytes = source.get(..8).ok_or(CodecError::IncompleteInput)?;
    let value = i64::from_be_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ]);
    *input = &source[8..];
    Ok(value)
}

/// Appends a 32-bit floating-point value in network (big-endian) byte order.
pub fn encode_f32(value: f32, output: &mut Vec<u8>) {
    output.extend_from_slice(&value.to_be_bytes());
}

/// Decodes a 32-bit network-order floating-point value transactionally.
pub fn decode_f32(input: &mut &[u8]) -> Result<f32, CodecError> {
    let source = *input;
    let bytes = source.get(..4).ok_or(CodecError::IncompleteInput)?;
    let value = f32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    *input = &source[4..];
    Ok(value)
}

/// Appends a 64-bit floating-point value in network (big-endian) byte order.
pub fn encode_f64(value: f64, output: &mut Vec<u8>) {
    output.extend_from_slice(&value.to_be_bytes());
}

/// Decodes a 64-bit network-order floating-point value transactionally.
pub fn decode_f64(input: &mut &[u8]) -> Result<f64, CodecError> {
    let source = *input;
    let bytes = source.get(..8).ok_or(CodecError::IncompleteInput)?;
    let value = f64::from_be_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ]);
    *input = &source[8..];
    Ok(value)
}

/// Appends a boolean as a single byte (0x00 = false, 0x01 = true).
pub fn encode_bool(value: bool, output: &mut Vec<u8>) {
    output.push(if value { 0x01 } else { 0x00 });
}

/// Decodes a boolean byte transactionally.
///
/// Returns an error if the byte is not 0x00 or 0x01.
pub fn decode_bool(input: &mut &[u8]) -> Result<bool, CodecError> {
    let source = *input;
    let byte = source.first().copied().ok_or(CodecError::IncompleteInput)?;
    let value = match byte {
        0x00 => false,
        0x01 => true,
        _ => return Err(CodecError::InvalidBoolean),
    };
    *input = &source[1..];
    Ok(value)
}

/// Appends a signed 8-bit integer.
pub fn encode_i8(value: i8, output: &mut Vec<u8>) {
    output.push(value as u8);
}

/// Decodes a signed 8-bit integer transactionally.
pub fn decode_i8(input: &mut &[u8]) -> Result<i8, CodecError> {
    let source = *input;
    let byte = source.first().copied().ok_or(CodecError::IncompleteInput)?;
    *input = &source[1..];
    Ok(byte as i8)
}

/// Appends an unsigned 8-bit integer.
pub fn encode_u8(value: u8, output: &mut Vec<u8>) {
    output.push(value);
}

/// Decodes an unsigned 8-bit integer transactionally.
pub fn decode_u8(input: &mut &[u8]) -> Result<u8, CodecError> {
    let source = *input;
    let byte = source.first().copied().ok_or(CodecError::IncompleteInput)?;
    *input = &source[1..];
    Ok(byte)
}

/// Appends a signed 32-bit integer in network (big-endian) byte order.
pub fn encode_i32(value: i32, output: &mut Vec<u8>) {
    output.extend_from_slice(&value.to_be_bytes());
}

/// Decodes a signed network-order 32-bit integer transactionally.
pub fn decode_i32(input: &mut &[u8]) -> Result<i32, CodecError> {
    let source = *input;
    let bytes = source.get(..4).ok_or(CodecError::IncompleteInput)?;
    let value = i32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    *input = &source[4..];
    Ok(value)
}

/// A 128-bit UUID represented as two signed 64-bit halves.
///
/// In the Minecraft protocol, UUIDs are sent as two network-order `i64`
/// values: the most-significant 64 bits followed by the least-significant
/// 64 bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Uuid {
    /// The most-significant 64 bits.
    pub most_significant: i64,
    /// The least-significant 64 bits.
    pub least_significant: i64,
}

impl Uuid {
    /// Creates a UUID from its two signed 64-bit halves.
    pub const fn new(most_significant: i64, least_significant: i64) -> Self {
        Self {
            most_significant,
            least_significant,
        }
    }

    /// Creates a UUID from 16 big-endian bytes.
    pub fn from_be_bytes(bytes: [u8; 16]) -> Self {
        let most_significant = i64::from_be_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]);
        let least_significant = i64::from_be_bytes([
            bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15],
        ]);
        Self::new(most_significant, least_significant)
    }

    /// Converts the UUID to 16 big-endian bytes.
    pub fn to_be_bytes(self) -> [u8; 16] {
        let mut bytes = [0u8; 16];
        bytes[..8].copy_from_slice(&self.most_significant.to_be_bytes());
        bytes[8..].copy_from_slice(&self.least_significant.to_be_bytes());
        bytes
    }
}

/// Appends a UUID as two network-order `i64` values.
pub fn encode_uuid(value: Uuid, output: &mut Vec<u8>) {
    encode_i64(value.most_significant, output);
    encode_i64(value.least_significant, output);
}

/// Decodes a UUID (two network-order `i64` values) transactionally.
pub fn decode_uuid(input: &mut &[u8]) -> Result<Uuid, CodecError> {
    let source = *input;
    let most_significant = decode_i64(input)?;
    let least_significant = match decode_i64(input) {
        Ok(value) => value,
        Err(CodecError::IncompleteInput) => {
            *input = source;
            return Err(CodecError::IncompleteInput);
        }
        Err(error) => return Err(error),
    };
    Ok(Uuid::new(most_significant, least_significant))
}

/// Maximum length of a Minecraft username in UTF-16 code units.
pub const MAX_USERNAME_LENGTH: usize = 16;

/// Maximum length of a chat component JSON string in UTF-16 code units.
pub const MAX_CHAT_COMPONENT_LENGTH: usize = 32767;

/// Maximum length of a server ID string in UTF-16 units.
pub const MAX_SERVER_ID_LENGTH: usize = 20;

/// Maximum length of a public key byte array.
pub const MAX_PUBLIC_KEY_LENGTH: usize = 512;

/// Maximum length of a verify token byte array.
pub const MAX_VERIFY_TOKEN_LENGTH: usize = 16;

/// Expected length of an AES-128 shared secret.
pub const SHARED_SECRET_LENGTH: usize = 16;

/// Appends a byte array prefixed by its length as a VarInt.
///
/// The length is bounded by `max_length`. On error, `output` is unchanged.
pub fn encode_byte_array(
    value: &[u8],
    max_length: usize,
    output: &mut Vec<u8>,
) -> Result<(), CodecError> {
    if value.len() > max_length {
        return Err(CodecError::StringTooLong);
    }
    let length = i32::try_from(value.len()).map_err(|_| CodecError::VarIntTooLong)?;
    encode_var_int(length, output);
    output.extend_from_slice(value);
    Ok(())
}

/// Decodes a length-prefixed byte array transactionally.
///
/// The length is bounded by `max_length`. On incomplete input, the input
/// is unchanged.
pub fn decode_byte_array<'a>(
    input: &mut &'a [u8],
    max_length: usize,
) -> Result<&'a [u8], CodecError> {
    let source = *input;
    let length = decode_var_int(input)?;
    let length = usize::try_from(length).map_err(|_| CodecError::VarIntTooLong)?;
    if length > max_length {
        *input = source;
        return Err(CodecError::StringTooLong);
    }
    let Some(bytes) = input.get(..length) else {
        *input = source;
        return Err(CodecError::IncompleteInput);
    };
    *input = &input[length..];
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::{
        CodecError, Uuid, decode_byte_array, decode_i64, decode_string, decode_u16, decode_uuid,
        decode_var_int, decode_var_long, encode_byte_array, encode_i64, encode_string, encode_u16,
        encode_uuid, encode_var_int, encode_var_long,
    };

    #[test]
    fn var_int_matches_public_examples() {
        let examples = [
            (0, &[0x00][..]),
            (1, &[0x01][..]),
            (2, &[0x02][..]),
            (127, &[0x7f][..]),
            (128, &[0x80, 0x01][..]),
            (255, &[0xff, 0x01][..]),
            (2_147_483_647, &[0xff, 0xff, 0xff, 0xff, 0x07][..]),
            (-1, &[0xff, 0xff, 0xff, 0xff, 0x0f][..]),
            (-2_147_483_648, &[0x80, 0x80, 0x80, 0x80, 0x08][..]),
        ];

        for (value, encoded) in examples {
            let mut output = Vec::new();
            encode_var_int(value, &mut output);
            assert_eq!(output, encoded);

            let mut input = encoded;
            assert_eq!(decode_var_int(&mut input), Ok(value));
            assert!(input.is_empty());
        }
    }

    #[test]
    fn var_int_round_trips_representative_values() {
        for value in [
            i32::MIN,
            -1_000_000,
            -128,
            -1,
            0,
            1,
            127,
            128,
            255,
            2_097_151,
            i32::MAX,
        ] {
            let mut encoded = Vec::new();
            encode_var_int(value, &mut encoded);
            assert!(encoded.len() <= 5);
            let mut input = encoded.as_slice();
            assert_eq!(decode_var_int(&mut input), Ok(value));
            assert!(input.is_empty());
        }
    }

    #[test]
    fn var_int_accepts_non_minimal_and_truncated_high_payload_bits() {
        for (encoded, expected) in [
            (&[0x81, 0x00][..], 1),
            (&[0x80, 0x80, 0x80, 0x80, 0x10][..], 0),
            (&[0xff, 0xff, 0xff, 0xff, 0x7f][..], -1),
        ] {
            let mut input = encoded;
            assert_eq!(decode_var_int(&mut input), Ok(expected));
            assert!(input.is_empty());
        }
    }

    #[test]
    fn var_int_rejects_truncated_and_fifth_byte_continuation_transactionally() {
        for (encoded, expected) in [
            (&[0x80][..], CodecError::IncompleteInput),
            (
                &[0x80, 0x80, 0x80, 0x80, 0x80, 0x00][..],
                CodecError::VarIntTooLong,
            ),
        ] {
            let mut input = encoded;
            assert_eq!(decode_var_int(&mut input), Err(expected));
            assert_eq!(input, encoded);
        }
    }

    #[test]
    fn var_long_handles_full_width_and_boundaries() {
        let examples = [
            (0, &[0x00][..]),
            (127, &[0x7f][..]),
            (128, &[0x80, 0x01][..]),
            (
                i64::MAX,
                &[0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x7f][..],
            ),
            (
                -1,
                &[0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x01][..],
            ),
            (
                i64::MIN,
                &[0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x01][..],
            ),
        ];

        for (value, encoded) in examples {
            let mut output = Vec::new();
            encode_var_long(value, &mut output);
            assert_eq!(output, encoded);
            let mut input = encoded;
            assert_eq!(decode_var_long(&mut input), Ok(value));
            assert!(input.is_empty());
        }
    }

    #[test]
    fn var_long_round_trips_representative_values() {
        for value in [
            i64::MIN,
            i32::MIN as i64,
            -1,
            0,
            1,
            127,
            128,
            i32::MAX as i64,
            i64::MAX,
        ] {
            let mut encoded = Vec::new();
            encode_var_long(value, &mut encoded);
            assert!(encoded.len() <= 10);
            let mut input = encoded.as_slice();
            assert_eq!(decode_var_long(&mut input), Ok(value));
            assert!(input.is_empty());
        }
    }

    #[test]
    fn var_long_accepts_non_minimal_and_truncated_high_payload_bits() {
        let high_tenth_payload = &[0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x7f][..];
        let discarded_tenth_payload =
            &[0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x02][..];
        for (encoded, expected) in [
            (&[0x81, 0x00][..], 1),
            (discarded_tenth_payload, 0),
            (high_tenth_payload, -1),
        ] {
            let mut input = encoded;
            assert_eq!(decode_var_long(&mut input), Ok(expected));
            assert!(input.is_empty());
        }
    }

    #[test]
    fn var_long_rejects_truncated_and_tenth_byte_continuation_transactionally() {
        let truncated = &[0x80][..];
        let tenth_byte_continues = &[0x80; 10][..];

        for (encoded, expected) in [
            (truncated, CodecError::IncompleteInput),
            (tenth_byte_continues, CodecError::VarLongTooLong),
        ] {
            let mut input = encoded;
            assert_eq!(decode_var_long(&mut input), Err(expected));
            assert_eq!(input, encoded);
        }
    }

    #[test]
    fn string_round_trips_ascii_and_multibyte_utf8() -> Result<(), CodecError> {
        for (value, max_utf16_units) in [("", 0), ("status", 6), ("界é", 2)] {
            let mut encoded = Vec::new();
            encode_string(value, max_utf16_units, &mut encoded)?;
            let mut input = encoded.as_slice();
            assert_eq!(decode_string(&mut input, max_utf16_units), Ok(value));
            assert!(input.is_empty());
        }
        Ok(())
    }

    #[test]
    fn string_limit_counts_utf16_units_including_surrogate_pairs() -> Result<(), CodecError> {
        let emoji = "😀";
        let mut output = Vec::new();
        assert_eq!(
            encode_string(emoji, 1, &mut output),
            Err(CodecError::StringTooLong)
        );
        assert!(output.is_empty());

        encode_string(emoji, 2, &mut output)?;
        assert_eq!(output, &[0x04, 0xf0, 0x9f, 0x98, 0x80]);
        let mut input = output.as_slice();
        assert_eq!(decode_string(&mut input, 2), Ok(emoji));
        assert!(input.is_empty());

        let encoded = output.as_slice();
        let mut input = encoded;
        assert_eq!(decode_string(&mut input, 1), Err(CodecError::StringTooLong));
        assert_eq!(input, encoded);
        Ok(())
    }

    #[test]
    fn string_enforces_three_bytes_per_utf16_unit_bound() -> Result<(), CodecError> {
        let mut encoded = Vec::new();
        encode_string("界", 1, &mut encoded)?;
        assert_eq!(encoded, &[0x03, 0xe7, 0x95, 0x8c]);

        let four_byte_payload = &[0x04, b'a', b'b', b'c', b'd'][..];
        let mut input = four_byte_payload;
        assert_eq!(decode_string(&mut input, 1), Err(CodecError::StringTooLong));
        assert_eq!(input, four_byte_payload);
        Ok(())
    }

    #[test]
    fn string_rejects_negative_invalid_and_truncated_input_transactionally() {
        let cases = [
            (
                &[0xff, 0xff, 0xff, 0xff, 0x0f][..],
                CodecError::NegativeStringLength,
            ),
            (&[0x02, 0xc3, 0x28][..], CodecError::InvalidUtf8),
            (&[0x03, b'a', b'b'][..], CodecError::IncompleteInput),
            (&[0x80][..], CodecError::IncompleteInput),
        ];

        for (encoded, expected) in cases {
            let mut input = encoded;
            assert_eq!(decode_string(&mut input, 8), Err(expected));
            assert_eq!(input, encoded);
        }
    }

    #[test]
    fn network_order_u16_handles_boundaries_and_truncation() {
        for (value, encoded) in [
            (u16::MIN, [0x00, 0x00]),
            (0x1234, [0x12, 0x34]),
            (u16::MAX, [0xff, 0xff]),
        ] {
            let mut output = Vec::new();
            encode_u16(value, &mut output);
            assert_eq!(output, encoded);
            let with_tail = [encoded[0], encoded[1], 0xaa];
            let mut input = with_tail.as_slice();
            assert_eq!(decode_u16(&mut input), Ok(value));
            assert_eq!(input, &[0xaa]);
        }

        let encoded = &[0x12][..];
        let mut input = encoded;
        assert_eq!(decode_u16(&mut input), Err(CodecError::IncompleteInput));
        assert_eq!(input, encoded);
    }

    #[test]
    fn network_order_i64_handles_boundaries_and_truncation() {
        for value in [i64::MIN, -1, 0, 0x0102_0304_0506_0708, i64::MAX] {
            let mut output = Vec::new();
            encode_i64(value, &mut output);
            assert_eq!(output, value.to_be_bytes());
            let mut input = output.as_slice();
            assert_eq!(decode_i64(&mut input), Ok(value));
            assert!(input.is_empty());
        }

        let encoded = &[0; 7][..];
        let mut input = encoded;
        assert_eq!(decode_i64(&mut input), Err(CodecError::IncompleteInput));
        assert_eq!(input, encoded);
    }

    #[test]
    fn uuid_round_trips_and_preserves_byte_order() -> Result<(), CodecError> {
        let uuid = Uuid::new(0x0102_0304_0506_0708, -1);
        let mut output = Vec::new();
        encode_uuid(uuid, &mut output);
        assert_eq!(output.len(), 16);
        let mut input = output.as_slice();
        assert_eq!(decode_uuid(&mut input), Ok(uuid));
        assert!(input.is_empty());
        Ok(())
    }

    #[test]
    fn uuid_from_and_to_be_bytes_are_inverses() {
        let bytes = [
            0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0xff, 0xfe, 0xfd, 0xfc, 0xfb, 0xfa,
            0xf9, 0xf8,
        ];
        let uuid = Uuid::from_be_bytes(bytes);
        assert_eq!(uuid.to_be_bytes(), bytes);
    }

    #[test]
    fn uuid_truncated_input_is_incomplete() {
        let encoded = &[0; 15][..];
        let mut input = encoded;
        assert_eq!(decode_uuid(&mut input), Err(CodecError::IncompleteInput));
        assert_eq!(input, encoded);
    }

    #[test]
    fn byte_array_round_trips() -> Result<(), CodecError> {
        let data = [0x01, 0x02, 0x03, 0x04];
        let mut output = Vec::new();
        encode_byte_array(&data, 16, &mut output)?;
        let mut input = output.as_slice();
        assert_eq!(decode_byte_array(&mut input, 16)?, &data);
        assert!(input.is_empty());
        Ok(())
    }

    #[test]
    fn byte_array_empty_round_trips() -> Result<(), CodecError> {
        let data: [u8; 0] = [];
        let mut output = Vec::new();
        encode_byte_array(&data, 16, &mut output)?;
        let mut input = output.as_slice();
        assert_eq!(decode_byte_array(&mut input, 16)?, &data);
        assert!(input.is_empty());
        Ok(())
    }

    #[test]
    fn byte_array_oversized_is_rejected() {
        let data = [0xff; 17];
        let result = encode_byte_array(&data, 16, &mut Vec::new());
        assert!(result.is_err());
    }

    #[test]
    fn byte_array_truncated_is_incomplete() -> Result<(), CodecError> {
        let data = [0x01, 0x02, 0x03, 0x04];
        let mut output = Vec::new();
        encode_byte_array(&data, 16, &mut output)?;
        for split in 0..output.len() {
            let mut input = &output[..split];
            assert_eq!(
                decode_byte_array(&mut input, 16),
                Err(CodecError::IncompleteInput)
            );
            assert_eq!(input, &output[..split]);
        }
        Ok(())
    }
}
