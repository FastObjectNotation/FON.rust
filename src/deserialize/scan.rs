//! Byte-level scanners: locating delimiters and matching brackets/braces.

use memchr::memchr;

use crate::error::FonError;


pub(super) fn find_byte(data: &[u8], target: u8) -> Option<usize> {
    memchr(target, data)
}


// Length of the token at the start of `data`: everything up to the next comma, or all
// of `data` when there is none. The caller decides whether the token is well formed.
pub(super) fn token_end(data: &[u8]) -> usize {
    memchr(b',', data).unwrap_or(data.len())
}


pub(super) fn find_closing_bracket(data: &[u8]) -> Result<usize, FonError> {
    find_closing(data, b'[', b']')
        .ok_or_else(|| FonError::Parse("Closing bracket not found".into()))
}


pub(super) fn find_closing_brace(data: &[u8]) -> Result<usize, FonError> {
    find_closing(data, b'{', b'}')
        .ok_or_else(|| FonError::Parse("Closing brace not found".into()))
}


// Index of the delimiter that closes the first `open` at `data[0]`. Brackets inside
// a quoted string are ignored; inside a string a backslash escapes exactly the next
// byte, so `"C:\\"` closes after the second backslash instead of the quote being
// treated as escaped.
fn find_closing(data: &[u8], open: u8, close: u8) -> Option<usize> {
    let mut depth: i32 = 0;
    let mut in_string = false;
    let mut escaped = false;

    for (i, &c) in data.iter().enumerate() {
        if in_string {
            if escaped {
                escaped = false;
            } else if c == b'\\' {
                escaped = true;
            } else if c == b'"' {
                in_string = false;
            }
        } else if c == b'"' {
            in_string = true;
        } else if c == open {
            depth += 1;
        } else if c == close {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
    }
    None
}
