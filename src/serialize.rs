use std::fmt::Write as _;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use rayon::prelude::*;

use crate::error::FonError;
use crate::types::{FonCollection, FonDump, FonValue};


pub fn serialize_to_string(collection: &FonCollection) -> String {
    let mut out = String::with_capacity(4096);
    serialize_body(&mut out, collection);
    out
}


pub fn serialize_dump_lines(dump: &FonDump, _max_threads: i32) -> Vec<String> {
    let mut entries: Vec<(u64, &FonCollection)> = dump.iter().map(|(id, c)| (*id, c)).collect();
    entries.sort_by_key(|(id, _)| *id);

    entries
        .par_iter()
        .map(|(_, c)| serialize_to_string(c))
        .collect()
}


pub fn serialize_dump_to_string(dump: &FonDump, max_threads: i32) -> String {
    let lines = serialize_dump_lines(dump, max_threads);
    let total: usize = lines.iter().map(|l| l.len() + 1).sum();
    let mut out = String::with_capacity(total);
    for line in &lines {
        out.push_str(line);
        out.push('\n');
    }
    out
}


pub fn serialize_to_file(dump: &FonDump, path: &Path, max_threads: i32) -> Result<(), FonError> {
    let lines = serialize_dump_lines(dump, max_threads);

    let file = File::create(path)
        .map_err(|e| FonError::Write(format!("Failed to open file for writing: {}", e)))?;
    let mut writer = BufWriter::new(file);
    for line in &lines {
        writer
            .write_all(line.as_bytes())
            .map_err(|e| FonError::Write(format!("Write failed: {}", e)))?;
        writer
            .write_all(b"\n")
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
            out.push('"');
            // Raw is logically lazy-encoded: emit the encoded form if already
            // packed, otherwise pack a copy (we only borrow `raw` here).
            if raw.is_packed() {
                out.push_str(raw.encoded());
            } else if raw.is_unpacked() {
                let mut tmp = crate::raw_data::RawData::from_bytes(raw.data().to_vec());
                tmp.pack();
                out.push_str(tmp.encoded());
            }
            out.push('"');
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
