use std::fs;
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;

use memchr::{memchr, memchr2};
use rayon::prelude::*;

use crate::error::FonError;
use crate::raw_data::RawData;
use crate::types::{
    FonCollection, FonDump, FonValue, TYPE_BOOL, TYPE_BYTE, TYPE_DOUBLE, TYPE_FLOAT, TYPE_INT,
    TYPE_LONG, TYPE_OBJECT, TYPE_RAW, TYPE_SHORT, TYPE_STRING, TYPE_UINT, TYPE_ULONG,
};


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


// Parse every non-empty line in one newline-aligned chunk. Mirrors the old
// single-threaded splitter: '\n', '\r', and '\r\n' all terminate a line, empty
// lines and empty-parsed collections are dropped.
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


fn parse_collection_body<'a>(
    line: &'a [u8],
    depth: i32,
    opts: &DeserializeOptions,
    interner: &mut KeyInterner<'a>,
) -> Result<FonCollection, FonError> {
    let mut collection = FonCollection::new();
    let mut pos = 0;

    while pos < line.len() {
        let eq_pos = match find_byte(&line[pos..], b'=') {
            Some(p) => pos + p,
            None => break,
        };

        let key_str = as_str(&line[pos..eq_pos]);
        let key = intern(interner, key_str);
        pos = eq_pos + 1;

        if pos >= line.len() || pos + 1 >= line.len() || line[pos + 1] != b':' {
            return Err(FonError::Parse(
                "Invalid format: expected type:value".into(),
            ));
        }

        let type_char = line[pos];
        pos += 2;

        let remaining = &line[pos..];
        let (value, consumed) = parse_value(remaining, type_char, depth, opts, interner)?;
        collection.add(key, value);
        pos += consumed;

        if pos < line.len() && line[pos] == b',' {
            pos += 1;
        }
    }

    Ok(collection)
}


fn parse_value<'a>(
    data: &'a [u8],
    type_char: u8,
    depth: i32,
    opts: &DeserializeOptions,
    interner: &mut KeyInterner<'a>,
) -> Result<(FonValue, usize), FonError> {
    if data.is_empty() {
        return Err(FonError::Parse("Empty value".into()));
    }

    if type_char == TYPE_OBJECT {
        if data[0] == b'{' {
            let (obj, consumed) = parse_object(data, depth + 1, opts, interner)?;
            return Ok((FonValue::Object(obj), consumed));
        }
        if data[0] == b'[' {
            let (arr, consumed) = parse_object_array(data, depth + 1, opts, interner)?;
            return Ok((FonValue::ObjectArray(arr), consumed));
        }
        return Err(FonError::Parse("Object must start with '{' or '['".into()));
    }

    if data[0] == b'[' {
        return match type_char {
            TYPE_BYTE => parse_number_array::<u8>(data).map(|(v, c)| (FonValue::ByteArray(v), c)),
            TYPE_SHORT => {
                parse_number_array::<i16>(data).map(|(v, c)| (FonValue::ShortArray(v), c))
            }
            TYPE_INT => parse_number_array::<i32>(data).map(|(v, c)| (FonValue::IntArray(v), c)),
            TYPE_UINT => parse_number_array::<u32>(data).map(|(v, c)| (FonValue::UIntArray(v), c)),
            TYPE_LONG => parse_number_array::<i64>(data).map(|(v, c)| (FonValue::LongArray(v), c)),
            TYPE_ULONG => {
                parse_number_array::<u64>(data).map(|(v, c)| (FonValue::ULongArray(v), c))
            }
            TYPE_FLOAT => {
                parse_number_array::<f32>(data).map(|(v, c)| (FonValue::FloatArray(v), c))
            }
            TYPE_DOUBLE => {
                parse_number_array::<f64>(data).map(|(v, c)| (FonValue::DoubleArray(v), c))
            }
            _ => Err(FonError::Parse("Unsupported array type".into())),
        };
    }

    if type_char == TYPE_STRING {
        let (s, consumed) = parse_string(data)?;
        return Ok((FonValue::String(s), consumed));
    }

    if type_char == TYPE_RAW {
        let (s, consumed) = parse_string(data)?;
        let mut raw = RawData::from_encoded(s);
        if opts.unpack_raw {
            raw.unpack()?;
        }
        return Ok((FonValue::Raw(Box::new(raw)), consumed));
    }

    let end = find_value_end(data);
    let value_str = as_str(&data[..end]);
    let mut consumed = end;
    if consumed < data.len() && data[consumed] == b',' {
        consumed += 1;
    }

    let value = match type_char {
        TYPE_BYTE => FonValue::Byte(parse_num::<u8>(value_str)?),
        TYPE_SHORT => FonValue::Short(parse_num::<i16>(value_str)?),
        TYPE_INT => FonValue::Int(parse_num::<i32>(value_str)?),
        TYPE_UINT => FonValue::UInt(parse_num::<u32>(value_str)?),
        TYPE_LONG => FonValue::Long(parse_num::<i64>(value_str)?),
        TYPE_ULONG => FonValue::ULong(parse_num::<u64>(value_str)?),
        TYPE_FLOAT => FonValue::Float(parse_num::<f32>(value_str)?),
        TYPE_DOUBLE => FonValue::Double(parse_num::<f64>(value_str)?),
        TYPE_BOOL => FonValue::Bool(data[0] != b'0'),
        _ => return Err(FonError::Parse("Unknown type".into())),
    };

    Ok((value, consumed))
}


fn parse_num<T: FromStr>(s: &str) -> Result<T, FonError> {
    s.parse::<T>()
        .map_err(|_| FonError::Parse(format!("Failed to parse number: '{}'", s)))
}


fn parse_number_array<T: FromStr>(data: &[u8]) -> Result<(Vec<T>, usize), FonError> {
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
        let s = as_str(&remaining[..end]);
        result.push(parse_num::<T>(s)?);
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


fn parse_string(data: &[u8]) -> Result<(String, usize), FonError> {
    if data[0] != b'"' {
        return Err(FonError::Parse("String must start with '\"'".into()));
    }

    let mut end_quote = 1;
    while end_quote < data.len() {
        if data[end_quote] == b'"' && data[end_quote - 1] != b'\\' {
            break;
        }
        end_quote += 1;
    }

    let content = &data[1..end_quote];

    // Fast path: no escapes.
    if !content.contains(&b'\\') {
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


fn find_value_end(data: &[u8]) -> usize {
    for (i, &c) in data.iter().enumerate() {
        if c == b',' || c == b']' || c == b'\r' || c == b'\n' {
            return i;
        }
    }
    data.len()
}


fn find_closing_bracket(data: &[u8]) -> Result<usize, FonError> {
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


fn find_closing_brace(data: &[u8]) -> Result<usize, FonError> {
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


fn parse_object<'a>(
    data: &'a [u8],
    depth: i32,
    opts: &DeserializeOptions,
    interner: &mut KeyInterner<'a>,
) -> Result<(Box<FonCollection>, usize), FonError> {
    if depth > opts.max_depth {
        return Err(FonError::Parse("Maximum nesting depth exceeded".into()));
    }
    if data[0] != b'{' {
        return Err(FonError::Parse("Object must start with '{'".into()));
    }

    let close = find_closing_brace(data)?;
    let body = &data[1..close];

    let collection = parse_collection_body(body, depth, opts, interner)?;

    let mut consumed = close + 1;
    if consumed < data.len() && data[consumed] == b',' {
        consumed += 1;
    }

    Ok((Box::new(collection), consumed))
}


#[allow(clippy::vec_box)] // Vec<Box<FonCollection>> matches FonValue::ObjectArray variant
fn parse_object_array<'a>(
    data: &'a [u8],
    depth: i32,
    opts: &DeserializeOptions,
    interner: &mut KeyInterner<'a>,
) -> Result<(Vec<Box<FonCollection>>, usize), FonError> {
    if depth > opts.max_depth {
        return Err(FonError::Parse("Maximum nesting depth exceeded".into()));
    }
    if data[0] != b'[' {
        return Err(FonError::Parse("Object array must start with '['".into()));
    }

    let close = find_closing_bracket(data)?;
    let content = &data[1..close];

    let mut result: Vec<Box<FonCollection>> = Vec::new();
    let mut pos = 0;
    while pos < content.len() {
        let remaining = &content[pos..];
        if remaining[0] != b'{' {
            return Err(FonError::Parse(
                "Object array element must start with '{'".into(),
            ));
        }
        let (obj, consumed) = parse_object(remaining, depth, opts, interner)?;
        result.push(obj);
        pos += consumed;
    }

    let mut total_consumed = close + 1;
    if total_consumed < data.len() && data[total_consumed] == b',' {
        total_consumed += 1;
    }

    Ok((result, total_consumed))
}


fn find_byte(data: &[u8], target: u8) -> Option<usize> {
    memchr(target, data)
}
