use std::fmt::Write as _;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use rayon::prelude::*;

use crate::error::FonError;
use crate::pool;
use crate::types::{FonCollection, FonDump, FonValue};


pub fn serialize_to_string(collection: &FonCollection) -> String {
    let mut out = String::new();
    serialize_body(&mut out, collection);
    out
}


// The live records in ascending id order. Dense and Slab stores already iterate in
// that order; only a Sparse store needs the sort.
fn ordered_records(dump: &FonDump) -> Vec<&FonCollection> {
    let mut entries: Vec<(u64, &FonCollection)> = dump.iter().collect();
    if !entries.windows(2).all(|pair| pair[0].0 < pair[1].0) {
        entries.sort_unstable_by_key(|(id, _)| *id);
    }
    entries.into_iter().map(|(_, c)| c).collect()
}


// One string per record, each sized to its text.
pub fn serialize_dump_lines(dump: &FonDump, max_threads: i32) -> Vec<String> {
    let records: Vec<&FonCollection> = ordered_records(dump);

    pool::run(max_threads, || {
        records
            .par_iter()
            .map_init(String::new, |scratch, c| {
                scratch.clear();
                serialize_body(scratch, c);
                scratch.as_str().to_owned()
            })
            .collect()
    })
}


// The dump text as a list of blocks, each holding a run of whole lines. Records are
// written straight into a block, so no per-record string is allocated.
fn serialize_dump_blocks(dump: &FonDump, max_threads: i32) -> Vec<String> {
    const MIN_BLOCK: usize = 64;
    const MAX_BLOCK: usize = 4096;
    let records: Vec<&FonCollection> = ordered_records(dump);

    pool::run(max_threads, || {
        let workers = rayon::current_num_threads().max(1);
        let block: usize = (records.len() / (workers * 8)).clamp(MIN_BLOCK, MAX_BLOCK);
        records
            .par_chunks(block)
            .map(|part| {
                let mut out = String::new();
                for c in part {
                    serialize_body(&mut out, c);
                    out.push('\n');
                }
                out
            })
            .collect()
    })
}


pub fn serialize_dump_to_string(dump: &FonDump, max_threads: i32) -> String {
    let blocks: Vec<String> = serialize_dump_blocks(dump, max_threads);
    let total: usize = blocks.iter().map(String::len).sum();
    let mut out = String::with_capacity(total);
    for block in &blocks {
        out.push_str(block);
    }
    out
}


/// Write the dump to `path`, replacing its contents. The file is written in place, so
/// a failure part way leaves a truncated file.
pub fn serialize_to_file(dump: &FonDump, path: &Path, max_threads: i32) -> Result<(), FonError> {
    let blocks: Vec<String> = serialize_dump_blocks(dump, max_threads);

    let file = File::create(path)
        .map_err(|e| FonError::Write(format!("Failed to open file for writing: {}", e)))?;
    let mut writer = BufWriter::new(file);
    for block in &blocks {
        writer
            .write_all(block.as_bytes())
            .map_err(|e| FonError::Write(format!("Write failed: {}", e)))?;
    }
    writer
        .flush()
        .map_err(|e| FonError::Write(format!("Flush failed: {}", e)))?;
    Ok(())
}


fn serialize_body(out: &mut String, collection: &FonCollection) {
    let mut first = true;
    for (key, value) in collection.iter() {
        if !first {
            out.push(',');
        }
        first = false;
        out.push_str(key);
        out.push('=');
        out.push(value.type_char() as char);
        out.push(':');
        serialize_value(out, value);
    }
}


fn serialize_value(out: &mut String, value: &FonValue) {
    match value {
        FonValue::Byte(v) => write!(out, "{}", v).unwrap(),
        FonValue::Short(v) => write!(out, "{}", v).unwrap(),
        FonValue::Int(v) => write!(out, "{}", v).unwrap(),
        FonValue::UInt(v) => write!(out, "{}", v).unwrap(),
        FonValue::Long(v) => write!(out, "{}", v).unwrap(),
        FonValue::ULong(v) => write!(out, "{}", v).unwrap(),
        FonValue::Float(v) => serialize_float(out, *v),
        FonValue::Double(v) => serialize_double(out, *v),
        FonValue::Bool(v) => out.push(if *v { '1' } else { '0' }),
        FonValue::String(s) => serialize_string(out, s),
        FonValue::Raw(raw) => {
            // Raw is logically lazy-encoded: emit the encoded form if already
            // packed, otherwise pack a copy (we only borrow `raw` here). Packed text
            // came from the caller or from the input and was never checked, so it is
            // written like any string: whatever it holds, the line stays parseable.
            if raw.is_packed() {
                serialize_string(out, raw.encoded());
            } else {
                let mut tmp = crate::raw_data::RawData::from_bytes(raw.data().to_vec());
                tmp.pack();
                serialize_string(out, tmp.encoded());
            }
        }
        FonValue::Object(child) => {
            out.push('{');
            serialize_body(out, child);
            out.push('}');
        }
        FonValue::ByteArray(arr) => serialize_int_array(out, arr),
        FonValue::ShortArray(arr) => serialize_int_array(out, arr),
        FonValue::IntArray(arr) => serialize_int_array(out, arr),
        FonValue::UIntArray(arr) => serialize_int_array(out, arr),
        FonValue::LongArray(arr) => serialize_int_array(out, arr),
        FonValue::ULongArray(arr) => serialize_int_array(out, arr),
        FonValue::FloatArray(arr) => {
            out.push('[');
            let mut first = true;
            for v in arr {
                if !first {
                    out.push(',');
                }
                first = false;
                serialize_float(out, *v);
            }
            out.push(']');
        }
        FonValue::DoubleArray(arr) => {
            out.push('[');
            let mut first = true;
            for v in arr {
                if !first {
                    out.push(',');
                }
                first = false;
                serialize_double(out, *v);
            }
            out.push(']');
        }
        FonValue::BoolArray(arr) => {
            out.push('[');
            for (i, v) in arr.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push(if *v { '1' } else { '0' });
            }
            out.push(']');
        }
        FonValue::StringArray(arr) => {
            out.push('[');
            for (i, s) in arr.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                serialize_string(out, s);
            }
            out.push(']');
        }
        FonValue::ObjectArray(arr) => {
            out.push('[');
            for (i, child) in arr.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push('{');
                serialize_body(out, child);
                out.push('}');
            }
            out.push(']');
        }
    }
}


fn serialize_int_array<T: std::fmt::Display>(out: &mut String, arr: &[T]) {
    out.push('[');
    for (i, v) in arr.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        write!(out, "{}", v).unwrap();
    }
    out.push(']');
}


fn serialize_float(out: &mut String, v: f32) {
    if v.is_nan() {
        out.push_str("nan");
    } else if v.is_infinite() {
        if v.is_sign_negative() {
            out.push_str("-inf");
        } else {
            out.push_str("inf");
        }
    } else {
        // Shortest representation that round-trips (Rust's default float Display).
        write!(out, "{}", v).unwrap();
    }
}


fn serialize_double(out: &mut String, v: f64) {
    if v.is_nan() {
        out.push_str("nan");
    } else if v.is_infinite() {
        if v.is_sign_negative() {
            out.push_str("-inf");
        } else {
            out.push_str("inf");
        }
    } else {
        write!(out, "{}", v).unwrap();
    }
}


fn serialize_string(out: &mut String, s: &str) {
    out.push('"');
    // Most strings need no escaping; copy those in one go.
    if !s.bytes().any(|b| b == b'"' || b == b'\\' || b < 0x20) {
        out.push_str(s);
        out.push('"');
        return;
    }
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{0008}' => out.push_str("\\b"),
            '\u{000C}' => out.push_str("\\f"),
            c if (c as u32) < 32 => {
                write!(out, "\\u{:04X}", c as u32).unwrap();
            }
            c => out.push(c),
        }
    }
    out.push('"');
}
