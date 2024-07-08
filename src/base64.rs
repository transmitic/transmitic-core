//! Base64 encoding and decoding
//!
//! # Examples
//!
//! ```
//! use transmitic_core::base64;  // Replace vendor_base64 with your project name
//!
//! // Encode
//! let bytes = "rust".as_bytes();
//! let encoded = base64::encode(bytes);
//! assert_eq!(encoded, "cnVzdA==");
//!
//! // Decode
//! let decoded = base64::decode(&encoded).unwrap();
//! assert_eq!(decoded, bytes);
//!
//! // Decode without padding
//! let decoded = base64::decode("cnVzdA").unwrap();
//! assert_eq!(decoded, bytes);
//! ```
use std::{error::Error, fmt};

const PADDING_CHAR: u8 = b'=';
const BIT_GROUP_LEN: usize = 6;
const BAD_IDX: u8 = 255; // Indicates an invalid base64 character

// Base64 lookup table
const TABLE: [u8; 64] = [
    b'A', b'B', b'C', b'D', b'E', b'F', b'G', b'H', b'I', b'J', b'K', b'L', b'M', b'N', b'O', b'P',
    b'Q', b'R', b'S', b'T', b'U', b'V', b'W', b'X', b'Y', b'Z', b'a', b'b', b'c', b'd', b'e', b'f',
    b'g', b'h', b'i', b'j', b'k', b'l', b'm', b'n', b'o', b'p', b'q', b'r', b's', b't', b'u', b'v',
    b'w', b'x', b'y', b'z', b'0', b'1', b'2', b'3', b'4', b'5', b'6', b'7', b'8', b'9', b'+', b'/',
];

// Each index corresponds to a value in TABLE. The value is the index of that value in TABLE.
// This could be replaced by a const HashMap when that becomes possible.
const TABLE_REVERSE: [u8; 123] = [
    BAD_IDX, BAD_IDX, BAD_IDX, BAD_IDX, BAD_IDX, BAD_IDX, BAD_IDX, BAD_IDX, BAD_IDX, BAD_IDX,
    BAD_IDX, BAD_IDX, BAD_IDX, BAD_IDX, BAD_IDX, BAD_IDX, BAD_IDX, BAD_IDX, BAD_IDX, BAD_IDX,
    BAD_IDX, BAD_IDX, BAD_IDX, BAD_IDX, BAD_IDX, BAD_IDX, BAD_IDX, BAD_IDX, BAD_IDX, BAD_IDX,
    BAD_IDX, BAD_IDX, BAD_IDX, BAD_IDX, BAD_IDX, BAD_IDX, BAD_IDX, BAD_IDX, BAD_IDX, BAD_IDX,
    BAD_IDX, BAD_IDX, BAD_IDX, 62, BAD_IDX, BAD_IDX, BAD_IDX, 63, 52, 53, 54, 55, 56, 57, 58, 59,
    60, 61, BAD_IDX, BAD_IDX, BAD_IDX, BAD_IDX, BAD_IDX, BAD_IDX, BAD_IDX, 0, 1, 2, 3, 4, 5, 6, 7,
    8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, BAD_IDX, BAD_IDX,
    BAD_IDX, BAD_IDX, BAD_IDX, BAD_IDX, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40,
    41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51,
];

/// Error that can occur when calling `decode()` if the string is not valid base64
#[derive(Clone)]
pub struct InvalidBase64String {
    invalid_byte: Option<u8>,
    index: usize,
}

impl Error for InvalidBase64String {}
impl fmt::Display for InvalidBase64String {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self.invalid_byte {
            Some(c) => {
                if c == PADDING_CHAR {
                    write!(
                        f,
                        "Base64 string contains padding character that is not at the end of the string. '{}' at index '{}'",
                        c as char, self.index,
                    )
                } else {
                    write!(
                        f,
                        "Base64 string contains character that is not valid base64. '{:#x}'/'{}' at index '{}'",
                        c, c as char, self.index,
                    )
                }
            }
            None => write!(
                f,
                "Base64 string is invalid length. Must be empty or at least 2 characters, that are not only padding characters.",
            ),
        }
    }
}

impl fmt::Debug for InvalidBase64String {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self.invalid_byte {
            Some(c) => {
                if c == PADDING_CHAR {
                    write!(
                        f,
                        "Base64 string contains padding character that is not at the end of the string. '{}' at index '{}'",
                        c as char, self.index,
                    )
                } else {
                    write!(
                        f,
                        "Base64 string contains character that is not valid base64. '{:#x}'/'{}' at index '{}'",
                        c, c as char, self.index,
                    )
                }
            }
            None => write!(
                f,
                "Base64 string is invalid length. Must be empty or at least 2 characters, that are not only padding characters.",
            ),
        }
    }
}

/// Encode bytes to base64 string.
///
/// # Examples
///
/// ```
/// use transmitic_core::base64;  // Replace vendor_base64 with your project name
///
/// let bytes = "rust".as_bytes();
/// let encoded = base64::encode(bytes);
/// assert_eq!(encoded, "cnVzdA==");
/// ```
#[allow(clippy::same_item_push)]
pub fn encode(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return "".to_string();
    }

    let mut output: Vec<u8> = Vec::with_capacity(get_encode_capacity(bytes));
    for i in (0..bytes.len()).step_by(3) {
        // 1st char
        let byte = bytes[i] >> 2;
        let output_byte = TABLE[byte as usize];
        output.push(output_byte);

        // 2nd char
        let mut byte = (bytes[i] & 0b11) << 4;
        let byte_2 = bytes.get(i + 1).unwrap_or(&0) >> 4;
        byte += byte_2;

        let output_byte = TABLE[byte as usize];
        output.push(output_byte);

        // 3rd char
        let mut byte = match bytes.get(i + 1) {
            Some(b) => (b & 0b1111) << 2,
            None => break,
        };
        let byte_2 = bytes.get(i + 2).unwrap_or(&0) >> 6;
        byte += byte_2;

        let output_byte = TABLE[byte as usize];
        output.push(output_byte);

        // 4th char
        let byte = match bytes.get(i + 2) {
            Some(b) => b & 0b111111,
            None => break,
        };

        let output_byte = TABLE[byte as usize];
        output.push(output_byte);
    }

    // Add padding
    for _ in 0..(output.capacity() - output.len()) {
        output.push(PADDING_CHAR);
    }

    debug_assert_eq!(output.len(), output.capacity());
    debug_assert_eq!(output.len(), get_encode_capacity(bytes));
    String::from_utf8(output).unwrap()
}

fn get_encode_capacity(input_bytes: &[u8]) -> usize {
    let input_bits = (input_bytes.len() * 8) as f64;
    let mut capacity: usize = ((input_bits / BIT_GROUP_LEN as f64).ceil()) as usize;

    let minimum_bytes = 4;
    let modd = capacity % minimum_bytes;
    if modd != 0 {
        capacity += minimum_bytes - modd;
    }

    capacity
}

/// Decode base64 string to bytes.  Handles padding and no padding.
///
/// # Examples
///
/// ```
/// use transmitic_core::base64;  // Replace vendor_base64 with your project name
///
/// // With padding
/// let string = "cnVzdA==";
/// let decoded = base64::decode(string).unwrap();
/// assert_eq!(decoded, [b'r', b'u', b's', b't']);
///
/// // Without padding
/// let string = "cnVzdA";
/// let decoded = base64::decode(string).unwrap();
/// assert_eq!(decoded, [b'r', b'u', b's', b't']);
/// ```
pub fn decode(base64_string: &str) -> Result<Vec<u8>, InvalidBase64String> {
    if base64_string.is_empty() {
        return Ok(Vec::new());
    }

    let string_bytes = base64_string.as_bytes();
    if string_bytes.len() == 1 {
        Err(InvalidBase64String {
            invalid_byte: None,
            index: 0,
        })?
    }
    if string_bytes.len() < 4
        && string_bytes[string_bytes.len() - 1] == PADDING_CHAR
        && string_bytes[string_bytes.len() - 2] == PADDING_CHAR
    {
        Err(InvalidBase64String {
            invalid_byte: None,
            index: 0,
        })?
    }

    let mut output = Vec::with_capacity(get_decode_capacity(string_bytes));
    let bytes_len = string_bytes.len();
    for i in (0..bytes_len).step_by(4) {
        let byte = string_bytes[i];
        let index_one = get_index_of_byte(byte, i)?;

        let byte = string_bytes[i + 1];
        let index_two = get_index_of_byte(byte, i + 1)?;

        // Byte 1
        let out_byte = (index_one << 2) + (index_two >> 4);
        output.push(out_byte);

        let byte = match string_bytes.get(i + 2) {
            Some(c) => *c,
            None => break,
        };
        if byte == PADDING_CHAR {
            // Check if padding is not at the end of string
            let byte_index = i + 2; // Factor in step_by
            if byte_index < bytes_len - 2 {
                Err(InvalidBase64String {
                    invalid_byte: Some(byte),
                    index: byte_index,
                })?
            }
            break;
        }
        let index_three = get_index_of_byte(byte, i + 2)?;

        // Byte 2
        let out_byte = ((index_two & 0b1111) << 4) + (index_three >> 2);
        output.push(out_byte);

        let byte = match string_bytes.get(i + 3) {
            Some(c) => *c,
            None => break,
        };
        if byte == PADDING_CHAR {
            // Check if padding is not at the end of string
            let byte_index = i + 3; // Factor in step_by
            if byte_index < bytes_len - 2 {
                Err(InvalidBase64String {
                    invalid_byte: Some(byte),
                    index: byte_index,
                })?
            }
            break;
        }
        let index_four = get_index_of_byte(byte, i + 3)?;

        // Byte 3
        let out_byte = ((index_three & 0b11) << 6) + index_four;
        output.push(out_byte);
    }

    debug_assert_eq!(output.len(), output.capacity());
    debug_assert_eq!(output.len(), get_decode_capacity(string_bytes));
    Ok(output)
}

fn get_index_of_byte(byte: u8, byte_index: usize) -> Result<u8, InvalidBase64String> {
    let index = match TABLE_REVERSE.get(byte as usize) {
        Some(i) => *i,
        None => Err(InvalidBase64String {
            invalid_byte: Some(byte),
            index: byte_index,
        })?,
    };
    if index == BAD_IDX {
        Err(InvalidBase64String {
            invalid_byte: Some(byte),
            index: byte_index,
        })?
    }

    Ok(index)
}

fn get_decode_capacity(string_bytes: &[u8]) -> usize {
    let mut capacity = string_bytes.len() * BIT_GROUP_LEN / 8;

    if string_bytes[string_bytes.len() - 1] == PADDING_CHAR {
        capacity -= 1;
    }
    if string_bytes[string_bytes.len() - 2] == PADDING_CHAR {
        capacity -= 1;
    }

    capacity
}

#[cfg(test)]
mod tests {
    use std::any::{Any, TypeId};

    use super::*;

    fn is_invalid_base64_string_error<T: ?Sized + Any>(_s: &T) -> bool {
        TypeId::of::<InvalidBase64String>() == TypeId::of::<T>()
    }

    #[test]
    fn test_1_char() {
        let input = "f".as_bytes();

        let encode_result = encode(input);
        assert_eq!(encode_result, "Zg==");

        let decode_result = decode(&encode_result).unwrap();
        assert_eq!(decode_result, input);
    }

    #[test]
    fn test_2_chars() {
        let input = "fo".as_bytes();

        let encode_result = encode(input);
        assert_eq!(encode_result, "Zm8=");

        let decode_result = decode(&encode_result).unwrap();
        assert_eq!(decode_result, input);
    }

    #[test]
    fn test_3_chars() {
        let input = "foo".as_bytes();

        let encode_result = encode(input);
        assert_eq!(encode_result, "Zm9v");

        let decode_result = decode(&encode_result).unwrap();
        assert_eq!(decode_result, input);
    }

    #[test]
    fn test_4_chars() {
        let input = "foob".as_bytes();

        let encode_result = encode(input);
        assert_eq!(encode_result, "Zm9vYg==");

        let decode_result = decode(&encode_result).unwrap();
        assert_eq!(decode_result, input);
    }

    #[test]
    fn test_5_chars() {
        let input = "fooba".as_bytes();

        let encode_result = encode(input);
        assert_eq!(encode_result, "Zm9vYmE=");

        let decode_result = decode(&encode_result).unwrap();
        assert_eq!(decode_result, input);
    }

    #[test]
    fn test_6_chars() {
        let input = "foobar".as_bytes();

        let encode_result = encode(input);
        assert_eq!(encode_result, "Zm9vYmFy");

        let decode_result = decode(&encode_result).unwrap();
        assert_eq!(decode_result, input);
    }

    #[test]
    fn test_rust() {
        let input = "rust".as_bytes();

        let encode_result = encode(input);
        assert_eq!(encode_result, "cnVzdA==");

        let decode_result = decode(&encode_result).unwrap();
        assert_eq!(decode_result, input);
    }

    #[test]
    fn test_1_char_is_padding() {
        let input = "=".as_bytes();

        let encode_result = encode(input);
        assert_eq!(encode_result, "PQ==");

        let decode_result = decode(&encode_result).unwrap();
        assert_eq!(decode_result, input);
    }

    #[test]
    fn test_2_chars_is_padding() {
        let input = "==".as_bytes();

        let encode_result = encode(input);
        assert_eq!(encode_result, "PT0=");

        let decode_result = decode(&encode_result).unwrap();
        assert_eq!(decode_result, input);
    }

    #[test]
    fn test_empty_string() {
        let input = "".as_bytes();

        let encode_result = encode(input);
        assert_eq!(encode_result, "");

        let decode_result = decode(&encode_result).unwrap();
        assert_eq!(decode_result, input);
    }

    #[test]
    fn test_zero_byte() {
        let input = &0b0u8.to_be_bytes();

        let encode_result = encode(input);
        assert_eq!(encode_result, "AA==");

        let decode_result = decode(&encode_result).unwrap();
        assert_eq!(decode_result, input);
    }

    #[test]
    fn test_long_string() {
        let input =
            "APokjfophrvuwh832749832Bblapfr][';;.vfheuUIOH]=32904-iJOHJDFOhgnfjgiuh82347^%^&s"
                .as_bytes();

        let encode_result = encode(input);
        assert_eq!(encode_result, "QVBva2pmb3BocnZ1d2g4MzI3NDk4MzJCYmxhcGZyXVsnOzsudmZoZXVVSU9IXT0zMjkwNC1pSk9ISkRGT2hnbmZqZ2l1aDgyMzQ3XiVeJnM=");

        let decode_result = decode(&encode_result).unwrap();
        assert_eq!(decode_result, input)
    }

    #[test]
    fn test_decode_missing_2_padding() {
        let input = "a".as_bytes();
        let decode_result = decode("YQ").unwrap();
        assert_eq!(decode_result, input);

        let decode_result_padding = decode("YQ==").unwrap();
        assert_eq!(decode_result, decode_result_padding);
    }

    #[test]
    fn test_decode_missing_1_padding() {
        let input = "ab".as_bytes();
        let decode_result = decode("YWI").unwrap();
        assert_eq!(decode_result, input);

        let decode_result_padding = decode("YWI=").unwrap();
        assert_eq!(decode_result, decode_result_padding);
    }

    #[test]
    fn test_decode_invalid_base64_string_1_char() {
        let decode_result = decode("b").unwrap_err();
        assert!(is_invalid_base64_string_error(&decode_result));
        assert_eq!(decode_result.index, 0);
        assert_eq!(decode_result.invalid_byte, None);
    }

    #[test]
    fn test_decode_invalid_base64_string_long() {
        let decode_result = decode("cnVzdGFj@@@@").unwrap_err();
        assert!(is_invalid_base64_string_error(&decode_result));
        assert_eq!(decode_result.index, 8);
        assert_eq!(decode_result.invalid_byte.unwrap(), b'@');
    }

    #[test]
    fn test_decode_all_base64_chars() {
        let all_chars =
            "1234567890qwertyuiopasdfghjklzxcvbnmQWERTYUIOPASDFGHJKLZXCVBNM+/".to_string();

        let decode_result = decode(&all_chars).unwrap();
        assert_eq!(decode_result, b"\xd7m\xf8\xe7\xae\xfc\xf7J\xb0z\xbbr\xba*)j\xc7_\x82\x18\xe4\x97<\\\xbd\xb9\xe6Aa\x11M\x85\x088\xf0\x12\x0cQ\x87$\xa2\xd9\\%A4\xcf\xbf");

        let encode_result = encode(&decode_result);
        assert_eq!(all_chars, encode_result);
    }

    #[test]
    fn test_decode_padding_in_the_middle_long() {
        let decode_result = decode("SSBhbSBhIHJ1c3RhY2Vhbi=u3c=").unwrap_err();
        assert!(is_invalid_base64_string_error(&decode_result));
        assert_eq!(decode_result.index, 22);
        assert_eq!(decode_result.invalid_byte.unwrap(), b'=');
    }

    #[test]
    fn test_decode_1_padding_char() {
        let decode_result = decode("=").unwrap_err();
        assert!(is_invalid_base64_string_error(&decode_result));
        assert_eq!(decode_result.index, 0);
    }

    #[test]
    fn test_decode_2_padding_chars() {
        let decode_result = decode("==").unwrap_err();
        assert!(is_invalid_base64_string_error(&decode_result));
        assert_eq!(decode_result.index, 0);
    }

    #[test]
    fn test_decode_3_padding_chars() {
        let decode_result = decode("===").unwrap_err();
        assert!(is_invalid_base64_string_error(&decode_result));
        assert_eq!(decode_result.index, 0);
    }

    #[test]
    fn test_decode_1_char_with_2_padding() {
        let decode_result = decode("a==").unwrap_err();
        assert!(is_invalid_base64_string_error(&decode_result));
        assert_eq!(decode_result.index, 0);
    }

    #[test]
    fn test_decode_padding_index_5() {
        let decode_result = decode("Zm9vY===").unwrap_err();
        assert!(is_invalid_base64_string_error(&decode_result));
        assert_eq!(decode_result.index, 5);
    }

    #[test]
    fn test_decode_padding_index_4() {
        let decode_result = decode("Zm9v====").unwrap_err();
        assert!(is_invalid_base64_string_error(&decode_result));
        assert_eq!(decode_result.index, 4);
    }

    #[test]
    fn test_decode_padding_index_3() {
        let decode_result = decode("Zm9=====").unwrap_err();
        assert!(is_invalid_base64_string_error(&decode_result));
        assert_eq!(decode_result.index, 3);
    }

    #[test]
    fn test_decode_padding_index_2() {
        let decode_result = decode("Zm======").unwrap_err();
        assert!(is_invalid_base64_string_error(&decode_result));
        assert_eq!(decode_result.index, 2);
    }

    #[test]
    fn test_decode_padding_index_1() {
        let decode_result = decode("Z=======").unwrap_err();
        assert!(is_invalid_base64_string_error(&decode_result));
        assert_eq!(decode_result.index, 1);
    }

    #[test]
    fn test_decode_padding_index_0() {
        let decode_result = decode("========").unwrap_err();
        assert!(is_invalid_base64_string_error(&decode_result));
        assert_eq!(decode_result.index, 0);
    }

    #[test]
    fn test_table_in_table_rev() {
        for (i, c) in TABLE.into_iter().enumerate() {
            assert_eq!(TABLE_REVERSE[c as usize], i as u8);
        }
    }
}
