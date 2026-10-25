//! Leaf value parsing: numbers (via the `atoi` crate for integers, std for
//! floats), numeric arrays, and quoted strings.

use memchr::memchr2;

use crate::error::FonError;

use super::as_str;
use super::scan::{find_closing_bracket, find_value_end};


pub(super) fn parse_num<T: NumParse>(bytes: &[u8]) -> Result<T, FonError> {
    T::parse_field(bytes)
}


// Numeric field parsing. Integers use the `atoi` crate's tight checked digit loop
// (faster than the standard library's generic `FromStr`); floats defer to the
// standard library, which already parses with the fast Eisel-Lemire algorithm.
pub(super) trait NumParse: Sized {
    fn parse_field(bytes: &[u8]) -> Result<Self, FonError>;
}


fn num_err(bytes: &[u8]) -> FonError {
    FonError::Parse(format!(
        "Failed to parse number: '{}'",
        String::from_utf8_lossy(bytes)
    ))
}


macro_rules! impl_num_parse_int {
    ($($t:ty),+) => {$(
        impl NumParse for $t {
            #[inline]
            fn parse_field(bytes: &[u8]) -> Result<Self, FonError> {
                atoi::atoi::<$t>(bytes).ok_or_else(|| num_err(bytes))
            }
        }
    )+};
}


macro_rules! impl_num_parse_float {
    ($($t:ty),+) => {$(
        impl NumParse for $t {
            #[inline]
            fn parse_field(bytes: &[u8]) -> Result<Self, FonError> {
                as_str(bytes).parse::<$t>().map_err(|_| num_err(bytes))
            }
        }
    )+};
}


impl_num_parse_int!(u8, u32, u64, i16, i32, i64);
impl_num_parse_float!(f32, f64);


pub(super) fn parse_number_array<T: NumParse>(data: &[u8]) -> Result<(Vec<T>, usize), FonError> {
    if data[0] != b'[' {
        return Err(FonError::Parse("Array must start with '['".into()));
    }

    let close = find_closing_bracket(data)?;
    let content = &data[1..close];

    let mut result: Vec<T> = Vec::with_capacity(content.len() / 4);
    let mut pos = 0;
    while pos < content.len() {
        let remaining = &content[pos..];
        let end = find_value_end(remaining);
        result.push(parse_num::<T>(&remaining[..end])?);
        pos += end;
        if pos < content.len() && content[pos] == b',' {
            pos += 1;
        }
    }

    let mut total_consumed = close + 1;
    if total_consumed < data.len() && data[total_consumed] == b',' {
        total_consumed += 1;
    }

    Ok((result, total_consumed))
}


// Decode the four hex digits of a `\uXXXX` escape. Surrogate code points and
// anything that is not exactly four hex digits yield `None`.
fn decode_unicode_escape(hex: &[u8]) -> Option<char> {
    let mut code: u32 = 0;
    for &b in hex {
        let digit = (b as char).to_digit(16)?;
        code = code * 16 + digit;
    }
    char::from_u32(code)
}


pub(super) fn parse_string(data: &[u8]) -> Result<(String, usize), FonError> {
    if data[0] != b'"' {
        return Err(FonError::Parse("String must start with '\"'".into()));
    }

    // SIMD-scan to the next quote or backslash. A backslash escapes exactly the
    // next byte, so an escaped quote never closes the string.
    let mut end_quote = 1;
    let mut has_escape = false;
    loop {
        match memchr2(b'"', b'\\', &data[end_quote..]) {
            Some(rel) => {
                end_quote += rel;
                if data[end_quote] == b'"' {
                    break;
                }
                has_escape = true;
                end_quote = (end_quote + 2).min(data.len());
            }
            None => return Err(FonError::Parse("Unterminated string".into())),
        }
    }

    let content = &data[1..end_quote];

    // Fast path: no escapes (tracked during the single scan above).
    if !has_escape {
        let s = as_str(content).to_owned();
        let mut consumed = end_quote + 1;
        if consumed < data.len() && data[consumed] == b',' {
            consumed += 1;
        }
        return Ok((s, consumed));
    }

    let mut bytes = Vec::with_capacity(content.len());
    let mut i = 0;
    while i < content.len() {
        if content[i] == b'\\' && i + 1 < content.len() {
            i += 1;
            match content[i] {
                b'"' => bytes.push(b'"'),
                b'\\' => bytes.push(b'\\'),
                b'n' => bytes.push(b'\n'),
                b'r' => bytes.push(b'\r'),
                b't' => bytes.push(b'\t'),
                b'b' => bytes.push(b'\x08'),
                b'f' => bytes.push(b'\x0C'),
                b'u' => {
                    let hex = content
                        .get(i + 1..i + 5)
                        .ok_or_else(|| FonError::Parse("Malformed \\u escape in string".into()))?;
                    let ch = decode_unicode_escape(hex)
                        .ok_or_else(|| FonError::Parse("Malformed \\u escape in string".into()))?;
                    let mut utf8 = [0u8; 4];
                    bytes.extend_from_slice(ch.encode_utf8(&mut utf8).as_bytes());
                    i += 4;
                }
                other => bytes.push(other),
            }
        } else {
            bytes.push(content[i]);
        }
        i += 1;
    }
    // SAFETY: `content` is valid UTF-8 and un-escaping only substitutes ASCII
    // control bytes or copies existing bytes, so the result stays valid UTF-8.
    let s = unsafe { String::from_utf8_unchecked(bytes) };

    let mut consumed = end_quote + 1;
    if consumed < data.len() && data[consumed] == b',' {
        consumed += 1;
    }
    Ok((s, consumed))
}


pub(super) fn parse_string_array(data: &[u8]) -> Result<(Vec<String>, usize), FonError> {
    if data[0] != b'[' {
        return Err(FonError::Parse("Array must start with '['".into()));
    }

    let close = find_closing_bracket(data)?;
    let content = &data[1..close];

    let mut result: Vec<String> = Vec::new();
    let mut pos = 0;
    while pos < content.len() {
        let remaining = &content[pos..];
        let (s, consumed) = parse_string(remaining)?;
        result.push(s);
        pos += consumed;
    }

    let mut total_consumed = close + 1;
    if total_consumed < data.len() && data[total_consumed] == b',' {
        total_consumed += 1;
    }

    Ok((result, total_consumed))
}


pub(super) fn parse_bool_array(data: &[u8]) -> Result<(Vec<bool>, usize), FonError> {
    if data[0] != b'[' {
        return Err(FonError::Parse("Array must start with '['".into()));
    }

    let close = find_closing_bracket(data)?;
    let content = &data[1..close];

    let mut result: Vec<bool> = Vec::with_capacity(content.len() / 2);
    let mut pos = 0;
    while pos < content.len() {
        let remaining = &content[pos..];
        let end = find_value_end(remaining);
        result.push(remaining[0] != b'0');
        pos += end;
        if pos < content.len() && content[pos] == b',' {
            pos += 1;
        }
    }

    let mut total_consumed = close + 1;
    if total_consumed < data.len() && data[total_consumed] == b',' {
        total_consumed += 1;
    }

    Ok((result, total_consumed))
}
