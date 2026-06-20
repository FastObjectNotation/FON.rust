//! Byte-level scanners: locating delimiters and matching brackets/braces.

use memchr::memchr;

use crate::error::FonError;


pub(super) fn find_byte(data: &[u8], target: u8) -> Option<usize> {
    memchr(target, data)
}


pub(super) fn find_value_end(data: &[u8]) -> usize {
    for (i, &c) in data.iter().enumerate() {
        if c == b',' || c == b']' || c == b'\r' || c == b'\n' {
            return i;
        }
    }
    data.len()
}


pub(super) fn find_closing_bracket(data: &[u8]) -> Result<usize, FonError> {
    let mut depth: i32 = 0;
    let mut in_string = false;

    for i in 0..data.len() {
        let c = data[i];
        if c == b'"' && (i == 0 || data[i - 1] != b'\\') {
            in_string = !in_string;
        } else if !in_string {
            if c == b'[' {
                depth += 1;
            } else if c == b']' {
                depth -= 1;
                if depth == 0 {
                    return Ok(i);
                }
            }
        }
    }
    Err(FonError::Parse("Closing bracket not found".into()))
}


pub(super) fn find_closing_brace(data: &[u8]) -> Result<usize, FonError> {
    let mut depth: i32 = 0;
    let mut in_string = false;

    for i in 0..data.len() {
        let c = data[i];
        if c == b'"' && (i == 0 || data[i - 1] != b'\\') {
            in_string = !in_string;
        } else if !in_string {
            if c == b'{' {
                depth += 1;
            } else if c == b'}' {
                depth -= 1;
                if depth == 0 {
                    return Ok(i);
                }
            }
        }
    }
    Err(FonError::Parse("Closing brace not found".into()))
}
