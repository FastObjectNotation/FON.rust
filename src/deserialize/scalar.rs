//! Leaf value parsing: numbers (via the `atoi` crate for integers, std for
//! floats), numeric arrays, and quoted strings.
//!
//! Every parser here consumes exactly one value and reports its length. The
//! separator after a value belongs to the caller, so a missing, doubled or
//! trailing comma is an error instead of being absorbed.

use atoi::FromRadix10SignedChecked;
use memchr::{memchr2, memchr_iter};

use crate::error::FonError;

use super::as_str;
use super::scan::{find_closing_bracket, token_end};


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


// The whole token must be the number: `12abc`, a lone sign and an empty token are
// errors. `atoi` alone stops at the first non-digit and reports what it read so far.
macro_rules! impl_num_parse_int {
    ($($t:ty),+) => {$(
        impl NumParse for $t {
            #[inline]
            fn parse_field(bytes: &[u8]) -> Result<Self, FonError> {
                let (value, used): (Option<$t>, usize) = <$t>::from_radix_10_signed_checked(bytes);
                let sign: usize = usize::from(matches!(bytes.first(), Some(b'+' | b'-')));
                match value {
                    Some(v) if used == bytes.len() && used > sign => Ok(v),
                    _ => Err(num_err(bytes)),
                }
            }
        }
    )+};
}


// A token that overflows to infinity (`1e400`) is an error; infinity has to be
// spelled out (`inf`, `-inf`).
macro_rules! impl_num_parse_float {
    ($($t:ty),+) => {$(
        impl NumParse for $t {
            #[inline]
            fn parse_field(bytes: &[u8]) -> Result<Self, FonError> {
                let value: $t = as_str(bytes).parse::<$t>().map_err(|_| num_err(bytes))?;
                if value.is_infinite() && !spells_infinity(bytes) {
                    return Err(num_err(bytes));
                }
                Ok(value)
            }
        }
    )+};
}


fn spells_infinity(bytes: &[u8]) -> bool {
    let unsigned: &[u8] = match bytes.first() {
        Some(b'+' | b'-') => &bytes[1..],
        _ => bytes,
    };
    matches!(unsigned.first(), Some(b'i' | b'I'))
}


impl_num_parse_int!(u8, u32, u64, i16, i32, i64);
impl_num_parse_float!(f32, f64);


// The text between the brackets of the array that starts at `data[0]`, plus the
// length of the whole array including both brackets.
fn array_content(data: &[u8]) -> Result<(&[u8], usize), FonError> {
    if data.first() != Some(&b'[') {
        return Err(FonError::Parse("Array must start with '['".into()));
    }
    let close: usize = find_closing_bracket(data)?;
    Ok((&data[1..close], close + 1))
}


// Split `content` at every comma and hand each token to `parse`. An empty token
// (doubled or trailing comma) reaches `parse` as an empty slice and fails there.
fn parse_tokens<T>(
    content: &[u8],
    mut parse: impl FnMut(&[u8]) -> Result<T, FonError>,
) -> Result<Vec<T>, FonError> {
    if content.is_empty() {
        return Ok(Vec::new());
    }
    let mut result: Vec<T> = Vec::with_capacity(memchr_iter(b',', content).count() + 1);
    let mut pos: usize = 0;
    loop {
        let end: usize = pos + token_end(&content[pos..]);
        result.push(parse(&content[pos..end])?);
        if end == content.len() {
            return Ok(result);
        }
        pos = end + 1;
    }
}


pub(super) fn parse_number_array<T: NumParse>(data: &[u8]) -> Result<(Vec<T>, usize), FonError> {
    let (content, consumed): (&[u8], usize) = array_content(data)?;
    let result: Vec<T> = parse_tokens(content, parse_num::<T>)?;
    Ok((result, consumed))
}


pub(super) fn parse_bool(bytes: &[u8]) -> Result<bool, FonError> {
    match bytes {
        b"0" => Ok(false),
        b"1" => Ok(true),
        _ => Err(FonError::Parse(format!(
            "Failed to parse bool (expected 0 or 1): '{}'",
            String::from_utf8_lossy(bytes)
        ))),
    }
}


pub(super) fn parse_bool_array(data: &[u8]) -> Result<(Vec<bool>, usize), FonError> {
    let (content, consumed): (&[u8], usize) = array_content(data)?;
    let result: Vec<bool> = parse_tokens(content, parse_bool)?;
    Ok((result, consumed))
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


// Parse the quoted string that starts at `data[0]`. Returns the text and the number
// of bytes consumed, up to and including the closing quote.
pub(super) fn parse_string(data: &[u8]) -> Result<(String, usize), FonError> {
    if data.first() != Some(&b'"') {
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
    let consumed: usize = end_quote + 1;

    // Fast path: no escapes (tracked during the single scan above).
    if !has_escape {
        return Ok((as_str(content).to_owned(), consumed));
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
                _ => return Err(FonError::Parse("Unknown escape sequence in string".into())),
            }
        } else {
            bytes.push(content[i]);
        }
        i += 1;
    }
    // SAFETY: `content` is valid UTF-8 and un-escaping only substitutes ASCII
    // control bytes or copies existing bytes, so the result stays valid UTF-8.
    let s = unsafe { String::from_utf8_unchecked(bytes) };

    Ok((s, consumed))
}


pub(super) fn parse_string_array(data: &[u8]) -> Result<(Vec<String>, usize), FonError> {
    let (content, consumed): (&[u8], usize) = array_content(data)?;

    let mut result: Vec<String> = Vec::new();
    let mut pos: usize = 0;
    while pos < content.len() {
        let (s, used): (String, usize) = parse_string(&content[pos..])?;
        result.push(s);
        pos += used;
        if pos < content.len() {
            pos = after_separator(content, pos)?;
        }
    }

    Ok((result, consumed))
}


// `content[pos]` must be the comma that separates two items, and another item must
// follow it. Returns the position of that next item.
pub(super) fn after_separator(content: &[u8], pos: usize) -> Result<usize, FonError> {
    if content[pos] != b',' {
        return Err(FonError::Parse("Expected ',' between values".into()));
    }
    if pos + 1 == content.len() {
        return Err(FonError::Parse("Trailing ','".into()));
    }
    Ok(pos + 1)
}
