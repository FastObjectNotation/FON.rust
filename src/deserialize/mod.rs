//! Deserialization entry points and parallel chunk orchestration.
//!
//! The file is split by responsibility: this module holds the public API plus
//! the newline-aligned chunking and the per-chunk key interner; [`record`] does
//! the recursive descent for one record; [`scalar`] parses leaf values; [`scan`]
//! holds the byte-level delimiter/bracket scanners.

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use memchr::{memchr2, memrchr2};
use rayon::prelude::*;

use crate::error::FonError;
use crate::pool;
use crate::types::{validate_key, FonCollection, FonDump};

mod record;
mod scalar;
mod scan;

use record::parse_collection_body;


/// The deepest nesting the parser accepts, whatever `max_depth` says. Parsing recurses
/// once per level, so an unbounded limit would let input overflow the stack. At this
/// depth parsing, writing, cloning and dropping a record fit a 1 MiB stack even in an
/// unoptimised build.
pub const MAX_NESTING_DEPTH: i32 = 128;


#[derive(Debug, Clone)]
pub struct DeserializeOptions {
    /// Deepest allowed nesting of objects and object arrays. Values above
    /// [`MAX_NESTING_DEPTH`] are treated as that limit.
    pub max_depth: i32,
    /// Decode raw values from Z85 to bytes while parsing. When `false` the text is kept
    /// and decoded on demand by [`crate::RawData::unpack`].
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


impl DeserializeOptions {
    fn depth_limit(&self) -> i32 {
        self.max_depth.min(MAX_NESTING_DEPTH)
    }
}


pub fn deserialize_from_file(
    path: &Path,
    max_threads: i32,
    opts: &DeserializeOptions,
) -> Result<FonDump, FonError> {
    let content = fs::read(path)
        .map_err(|e| FonError::Io(format!("Failed to read '{}': {}", path.display(), e)))?;
    deserialize_dump_from_bytes(&content, max_threads, opts)
}


/// Parse a whole dump. Every line is one record and its 0-based line index is its id,
/// so an empty line is an empty record. `\n`, `\r\n` and `\r` end a line; the last line
/// needs no terminator. A UTF-8 byte order mark at the start of the input is skipped.
pub fn deserialize_dump_from_bytes(
    content: &[u8],
    max_threads: i32,
    opts: &DeserializeOptions,
) -> Result<FonDump, FonError> {
    let content: &[u8] = strip_bom(content);
    if content.is_empty() {
        return Ok(FonDump::new());
    }

    // Split into newline-aligned byte ranges and parse them in parallel — no
    // single-threaded whole-file line scan. Aim for ~1 MB chunks: small enough
    // to stay cache-resident and to give Rayon plenty of work items to balance
    // uneven records across cores, but never fewer than the thread count.
    const TARGET_CHUNK: usize = 1 << 20;
    type Parsed = (Vec<(usize, usize)>, Vec<Result<Vec<FonCollection>, ChunkError>>);
    let (bounds, parts): Parsed = pool::run(max_threads, || {
        let workers = rayon::current_num_threads().max(1);
        let chunk_count = (content.len() / TARGET_CHUNK).max(workers);
        let bounds = chunk_bounds(content, chunk_count);
        let parts = bounds
            .par_iter()
            .map(|&(start, end)| parse_chunk(&content[start..end], opts))
            .collect();
        (bounds, parts)
    });

    // Chunks are in file order, so the first failed chunk holds the first bad line.
    let total: usize = parts.iter().map(|part| part.as_ref().map_or(0, Vec::len)).sum();
    let mut records: Vec<FonCollection> = Vec::with_capacity(total);
    for (part, &(start, _)) in parts.into_iter().zip(&bounds) {
        match part {
            Ok(list) => records.extend(list),
            Err(failure) => {
                let line: usize = line_number(content, start + failure.offset);
                return Err(at_line(failure.error, line));
            }
        }
    }
    Ok(FonDump::from_records(records))
}


/// Parse one record. One trailing line terminator and a leading UTF-8 byte order mark
/// are ignored; any other line break is an error, because a record is a single line.
pub fn deserialize_line(line: &[u8], opts: &DeserializeOptions) -> Result<FonCollection, FonError> {
    std::str::from_utf8(line).map_err(|_| FonError::Parse("Invalid UTF-8 in input".into()))?;
    let line: &[u8] = strip_line_end(strip_bom(line));
    if memchr2(b'\n', b'\r', line).is_some() {
        return Err(FonError::Parse(
            "A single record must not contain a line break".into(),
        ));
    }
    let mut interner = KeyInterner::new();
    parse_collection_body(line, 0, opts, &mut interner)
}


fn strip_bom(content: &[u8]) -> &[u8] {
    content.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(content)
}


fn strip_line_end(line: &[u8]) -> &[u8] {
    line.strip_suffix(b"\r\n")
        .or_else(|| line.strip_suffix(b"\n"))
        .or_else(|| line.strip_suffix(b"\r"))
        .unwrap_or(line)
}


// Cut `content` into `n` line-aligned byte ranges: each range begins right after a
// line terminator (or at 0) and ends right after one (or at EOF), so no record and
// no `\r\n` pair is ever split across two ranges.
fn chunk_bounds(content: &[u8], n: usize) -> Vec<(usize, usize)> {
    let len = content.len();
    if len == 0 || n <= 1 {
        return vec![(0, len)];
    }

    let mut points = Vec::with_capacity(n + 1);
    points.push(0usize);
    for i in 1..n {
        let approx = ((len as u128 * i as u128) / n as u128) as usize;
        let next = match memchr2(b'\n', b'\r', &content[approx..]) {
            Some(rel) => {
                let at = approx + rel;
                if content[at] == b'\r' && content.get(at + 1) == Some(&b'\n') {
                    at + 2
                } else {
                    at + 1
                }
            }
            None => len,
        };
        if *points.last().unwrap() < next {
            points.push(next);
        }
    }
    if *points.last().unwrap() != len {
        points.push(len);
    }
    points.windows(2).map(|w| (w[0], w[1])).collect()
}


// A parse failure inside a chunk: the byte offset of the line that failed, relative to
// the chunk start, and the error itself.
struct ChunkError {
    offset: usize,
    error: FonError,
}


// Parse every line of one line-aligned chunk into a record. '\n', '\r', and '\r\n'
// all terminate a line, and an empty line is an empty record.
fn parse_chunk(chunk: &[u8], opts: &DeserializeOptions) -> Result<Vec<FonCollection>, ChunkError> {
    // Validate the whole chunk as UTF-8 once so per-field parsing can skip it. When the
    // chunk is not valid, only the lines before the first bad byte are parsed, so that
    // an earlier syntax error is still the one reported.
    let invalid_from: Option<usize> = match std::str::from_utf8(chunk) {
        Ok(_) => None,
        Err(e) => {
            let bad: usize = e.valid_up_to();
            Some(memrchr2(b'\n', b'\r', &chunk[..bad]).map_or(0, |p| p + 1))
        }
    };
    let limit: usize = invalid_from.unwrap_or(chunk.len());

    let mut out = Vec::new();
    // One interner shared across every record in the chunk: each distinct key is
    // allocated once and reused for all records the worker parses.
    let mut interner = KeyInterner::new();
    let mut start = 0;
    // SIMD-scan to the next line terminator instead of inspecting every byte.
    while start < limit {
        let (line_end, next) = match memchr2(b'\n', b'\r', &chunk[start..limit]) {
            Some(rel) => {
                let nl = start + rel;
                if chunk[nl] == b'\r' && chunk.get(nl + 1) == Some(&b'\n') {
                    (nl, nl + 2)
                } else {
                    (nl, nl + 1)
                }
            }
            None => (limit, limit),
        };
        let coll = parse_collection_body(&chunk[start..line_end], 0, opts, &mut interner)
            .map_err(|error| ChunkError { offset: start, error })?;
        out.push(coll);
        start = next;
    }

    match invalid_from {
        Some(offset) => Err(ChunkError {
            offset,
            error: FonError::Parse("Invalid UTF-8 in input".into()),
        }),
        None => Ok(out),
    }
}


// 1-based number of the line that starts at `offset`.
fn line_number(content: &[u8], offset: usize) -> usize {
    let head = &content[..offset];
    let mut line = 1;
    let mut pos = 0;
    while let Some(rel) = memchr2(b'\n', b'\r', &head[pos..]) {
        let at = pos + rel;
        pos = if head[at] == b'\r' && head.get(at + 1) == Some(&b'\n') {
            at + 2
        } else {
            at + 1
        };
        line += 1;
    }
    line
}


fn at_line(error: FonError, line: usize) -> FonError {
    match error {
        FonError::Parse(message) => FonError::Parse(format!("line {line}: {message}")),
        other => other,
    }
}


// Keys repeat on every record (`id`, `name`, ...). Intern them so a repeated key
// is allocated once and then shared via cheap Arc clones, instead of a fresh
// String per field. A record schema usually has a handful of distinct keys, where
// a tiny linear-scan table beats a HashMap (no hashing on every key); past
// `SMALL_SCHEMA` distinct keys the table moves into a map so lookups stay constant.
struct KeyInterner<'a> {
    small: Vec<(&'a str, Arc<str>)>,
    large: HashMap<&'a str, Arc<str>>,
}


const SMALL_SCHEMA: usize = 32;


impl<'a> KeyInterner<'a> {
    fn new() -> Self {
        Self {
            small: Vec::new(),
            large: HashMap::new(),
        }
    }


    // A key is validated once, when it is first seen: parsed keys obey the same rules
    // as `FonCollection::add_field`.
    fn intern(&mut self, key: &'a str) -> Result<Arc<str>, FonError> {
        if self.large.is_empty() {
            for (existing, shared) in &self.small {
                if *existing == key {
                    return Ok(Arc::clone(shared));
                }
            }
        } else if let Some(shared) = self.large.get(key) {
            return Ok(Arc::clone(shared));
        }

        validate_key(key).map_err(|e| FonError::Parse(e.to_string()))?;
        let shared: Arc<str> = Arc::from(key);
        if self.large.is_empty() && self.small.len() < SMALL_SCHEMA {
            self.small.push((key, Arc::clone(&shared)));
        } else {
            if self.large.is_empty() {
                self.large.extend(self.small.drain(..));
            }
            self.large.insert(key, Arc::clone(&shared));
        }
        Ok(shared)
    }
}


// SAFETY for all callers: `parse_chunk` and `deserialize_line` validate the input as
// UTF-8 before any parsing (`parse_chunk` parses only the prefix it has validated), and
// every slice handed here is cut at an ASCII delimiter (`=`, `:`, `,`, `"`, `[`, `]`,
// `{`, `}`, CR, LF) — which can only land on a UTF-8 character boundary — so the bytes
// are always valid UTF-8.
#[inline]
fn as_str(bytes: &[u8]) -> &str {
    unsafe { std::str::from_utf8_unchecked(bytes) }
}
