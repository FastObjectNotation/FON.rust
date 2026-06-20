//! Deserialization entry points and parallel chunk orchestration.
//!
//! The file is split by responsibility: this module holds the public API plus
//! the newline-aligned chunking and the per-chunk key interner; [`record`] does
//! the recursive descent for one record; [`scalar`] parses leaf values; [`scan`]
//! holds the byte-level delimiter/bracket scanners.

use std::fs;
use std::path::Path;
use std::sync::Arc;

use memchr::memchr2;
use rayon::prelude::*;

use crate::error::FonError;
use crate::types::{FonCollection, FonDump};

mod record;
mod scalar;
mod scan;

use record::parse_collection_body;


pub struct DeserializeOptions {
    pub max_depth: i32,
    pub unpack_raw: bool,
}


impl Default for DeserializeOptions {
    fn default() -> Self {
        Self {
            max_depth: 64,
            unpack_raw: false,
        }
    }
}


pub fn deserialize_from_file(
    path: &Path,
    max_threads: i32,
    opts: &DeserializeOptions,
) -> Result<FonDump, FonError> {
    let content =
        fs::read(path).map_err(|e| FonError::Parse(format!("Failed to open file: {}", e)))?;
    deserialize_dump_from_bytes(&content, max_threads, opts)
}


pub fn deserialize_dump_from_bytes(
    content: &[u8],
    _max_threads: i32,
    opts: &DeserializeOptions,
) -> Result<FonDump, FonError> {
    // Strip a UTF-8 BOM once so it never glues onto the first key.
    let content = if content.len() >= 3
        && content[0] == 0xEF
        && content[1] == 0xBB
        && content[2] == 0xBF
    {
        &content[3..]
    } else {
        content
    };
    if content.is_empty() {
        return Ok(FonDump::new());
    }

    // Split into newline-aligned byte ranges and parse them in parallel — no
    // single-threaded whole-file line scan. Aim for ~1 MB chunks: small enough
    // to stay cache-resident and to give Rayon plenty of work items to balance
    // uneven records across cores, but never fewer than the thread count.
    const TARGET_CHUNK: usize = 1 << 20;
    let workers = rayon::current_num_threads().max(1);
    let chunk_count = (content.len() / TARGET_CHUNK).max(workers);
    let bounds = chunk_bounds(content, chunk_count);
    let parts: Vec<Result<Vec<FonCollection>, FonError>> = bounds
        .par_iter()
        .map(|&(start, end)| parse_chunk(&content[start..end], opts))
        .collect();

    let mut records: Vec<FonCollection> = Vec::with_capacity(content.len() / 64);
    for part in parts {
        records.extend(part?);
    }
    Ok(FonDump::from_records(records))
}


pub fn deserialize_line(line: &[u8], opts: &DeserializeOptions) -> Result<FonCollection, FonError> {
    std::str::from_utf8(line).map_err(|_| FonError::Parse("Invalid UTF-8 in input".into()))?;
    let mut interner = KeyInterner::new();
    parse_collection_body(line, 0, opts, &mut interner)
}


// Cut `content` into `n` newline-aligned byte ranges: each range begins right
// after a '\n' (or at 0) and ends right after a '\n' (or at EOF), so no record
// is ever split across two ranges.
fn chunk_bounds(content: &[u8], n: usize) -> Vec<(usize, usize)> {
    let len = content.len();
    if len == 0 || n <= 1 {
        return vec![(0, len)];
    }

    let mut points = Vec::with_capacity(n + 1);
    points.push(0usize);
    for i in 1..n {
        let approx = (len * i / n).min(len);
        let next = content[approx..]
            .iter()
            .position(|&b| b == b'\n')
            .map(|p| approx + p + 1)
            .unwrap_or(len);
        if *points.last().unwrap() < next {
            points.push(next);
        }
    }
    if *points.last().unwrap() != len {
        points.push(len);
    }
    points.windows(2).map(|w| (w[0], w[1])).collect()
}


// Parse every non-empty line in one newline-aligned chunk. '\n', '\r', and '\r\n'
// all terminate a line; empty lines and empty-parsed collections are dropped.
fn parse_chunk(chunk: &[u8], opts: &DeserializeOptions) -> Result<Vec<FonCollection>, FonError> {
    // Validate the whole chunk as UTF-8 once so per-field parsing can skip it.
    std::str::from_utf8(chunk).map_err(|_| FonError::Parse("Invalid UTF-8 in input".into()))?;

    let mut out = Vec::new();
    // One interner shared across every record in the chunk: each distinct key is
    // allocated once and reused for all records the worker parses.
    let mut interner = KeyInterner::new();
    let len = chunk.len();
    let mut start = 0;
    // SIMD-scan to the next line terminator instead of inspecting every byte.
    while start < len {
        match memchr2(b'\n', b'\r', &chunk[start..]) {
            Some(rel) => {
                let nl = start + rel;
                if nl > start {
                    let coll = parse_collection_body(&chunk[start..nl], 0, opts, &mut interner)?;
                    if !coll.is_empty() {
                        out.push(coll);
                    }
                }
                start = if chunk[nl] == b'\r' && nl + 1 < len && chunk[nl + 1] == b'\n' {
                    nl + 2
                } else {
                    nl + 1
                };
            }
            None => {
                let coll = parse_collection_body(&chunk[start..], 0, opts, &mut interner)?;
                if !coll.is_empty() {
                    out.push(coll);
                }
                break;
            }
        }
    }
    Ok(out)
}


// A record schema has only a handful of distinct keys, so a tiny linear-scan
// table interns them faster than a HashMap: no hashing on every key, no per-chunk
// map allocation.
type KeyInterner<'a> = Vec<(&'a str, Arc<str>)>;


// Keys repeat on every record (`id`, `name`, ...). Intern them so a repeated key
// is allocated once and then shared via cheap Arc clones, instead of a fresh
// String per field.
fn intern<'a>(interner: &mut KeyInterner<'a>, key: &'a str) -> Arc<str> {
    for (existing, shared) in interner.iter() {
        if *existing == key {
            return Arc::clone(shared);
        }
    }
    let shared: Arc<str> = Arc::from(key);
    interner.push((key, Arc::clone(&shared)));
    shared
}


// SAFETY for all callers: `parse_chunk` and `deserialize_line` validate the whole
// input as UTF-8 before any parsing, and every slice handed here is cut at an
// ASCII delimiter (`=`, `:`, `,`, `"`, `[`, `]`) — which can only land on a UTF-8
// character boundary — so the bytes are always valid UTF-8.
#[inline]
fn as_str(bytes: &[u8]) -> &str {
    unsafe { std::str::from_utf8_unchecked(bytes) }
}
