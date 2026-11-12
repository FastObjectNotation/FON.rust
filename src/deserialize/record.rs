//! Recursive-descent parsing of one record (a line) into a `FonCollection`,
//! including nested objects and object arrays.

use std::collections::HashSet;
use std::sync::Arc;

use crate::error::FonError;
use crate::raw_data::RawData;
use crate::types::{
    FonCollection, FonValue, TYPE_BOOL, TYPE_BYTE, TYPE_DOUBLE, TYPE_FLOAT, TYPE_INT, TYPE_LONG,
    TYPE_OBJECT, TYPE_RAW, TYPE_SHORT, TYPE_STRING, TYPE_UINT, TYPE_ULONG,
};

use super::scalar::{
    after_separator, parse_bool, parse_bool_array, parse_num, parse_number_array, parse_string,
    parse_string_array,
};
use super::scan::{find_byte, find_closing_brace, find_closing_bracket, token_end};
use super::{as_str, DeserializeOptions, KeyInterner};


// Above this many fields the duplicate-key check switches from a linear scan to a set,
// so a very wide record stays linear to parse.
const WIDE_RECORD: usize = 32;


// Parse `key=type:value` pairs separated by single commas. An empty `line` is an
// empty collection. Anything else must be made of complete pairs: text with no `=`,
// a doubled or trailing comma and a repeated key are errors.
pub(super) fn parse_collection_body<'a>(
    line: &'a [u8],
    depth: i32,
    opts: &DeserializeOptions,
    interner: &mut KeyInterner<'a>,
) -> Result<FonCollection, FonError> {
    let mut collection = FonCollection::new();
    let mut seen: Option<HashSet<usize>> = None;
    let mut pos = 0;

    while pos < line.len() {
        let eq_pos = match find_byte(&line[pos..], b'=') {
            Some(p) => pos + p,
            None => {
                return Err(FonError::Parse(
                    "Invalid format: expected key=type:value".into(),
                ));
            }
        };

        let key: Arc<str> = interner.intern(as_str(&line[pos..eq_pos]))?;
        if is_repeated(&collection, &mut seen, &key) {
            return Err(FonError::Parse(format!("Duplicate key '{key}'")));
        }
        pos = eq_pos + 1;

        if pos + 1 >= line.len() || line[pos + 1] != b':' {
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

        if pos < line.len() {
            pos = after_separator(line, pos)?;
        }
    }

    Ok(collection)
}


// Keys are interned, so two fields with the same key share one allocation and the
// check is a pointer comparison.
fn is_repeated(collection: &FonCollection, seen: &mut Option<HashSet<usize>>, key: &Arc<str>) -> bool {
    let address = |k: &Arc<str>| -> usize { Arc::as_ptr(k) as *const u8 as usize };
    if collection.len() < WIDE_RECORD {
        return collection.iter().any(|(existing, _)| Arc::ptr_eq(existing, key));
    }
    let set: &mut HashSet<usize> =
        seen.get_or_insert_with(|| collection.iter().map(|(existing, _)| address(existing)).collect());
    !set.insert(address(key))
}


// Parse one value at the start of `data`. Returns the value and its length; the
// separator that follows is left for the caller.
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
            TYPE_STRING => parse_string_array(data).map(|(v, c)| (FonValue::StringArray(v), c)),
            TYPE_BOOL => parse_bool_array(data).map(|(v, c)| (FonValue::BoolArray(v), c)),
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

    let consumed: usize = token_end(data);
    let token = &data[..consumed];

    let value = match type_char {
        TYPE_BYTE => FonValue::Byte(parse_num::<u8>(token)?),
        TYPE_SHORT => FonValue::Short(parse_num::<i16>(token)?),
        TYPE_INT => FonValue::Int(parse_num::<i32>(token)?),
        TYPE_UINT => FonValue::UInt(parse_num::<u32>(token)?),
        TYPE_LONG => FonValue::Long(parse_num::<i64>(token)?),
        TYPE_ULONG => FonValue::ULong(parse_num::<u64>(token)?),
        TYPE_FLOAT => FonValue::Float(parse_num::<f32>(token)?),
        TYPE_DOUBLE => FonValue::Double(parse_num::<f64>(token)?),
        TYPE_BOOL => FonValue::Bool(parse_bool(token)?),
        _ => return Err(FonError::Parse("Unknown type".into())),
    };

    Ok((value, consumed))
}


fn parse_object<'a>(
    data: &'a [u8],
    depth: i32,
    opts: &DeserializeOptions,
    interner: &mut KeyInterner<'a>,
) -> Result<(Box<FonCollection>, usize), FonError> {
    if depth > opts.depth_limit() {
        return Err(FonError::Parse("Maximum nesting depth exceeded".into()));
    }
    if data.first() != Some(&b'{') {
        return Err(FonError::Parse("Object must start with '{'".into()));
    }

    let close = find_closing_brace(data)?;
    let body = &data[1..close];

    let collection = parse_collection_body(body, depth, opts, interner)?;

    Ok((Box::new(collection), close + 1))
}


#[allow(clippy::vec_box)] // Vec<Box<FonCollection>> matches FonValue::ObjectArray variant
fn parse_object_array<'a>(
    data: &'a [u8],
    depth: i32,
    opts: &DeserializeOptions,
    interner: &mut KeyInterner<'a>,
) -> Result<(Vec<Box<FonCollection>>, usize), FonError> {
    if depth > opts.depth_limit() {
        return Err(FonError::Parse("Maximum nesting depth exceeded".into()));
    }
    if data.first() != Some(&b'[') {
        return Err(FonError::Parse("Object array must start with '['".into()));
    }

    let close = find_closing_bracket(data)?;
    let content = &data[1..close];

    let mut result: Vec<Box<FonCollection>> = Vec::new();
    let mut pos = 0;
    while pos < content.len() {
        if content[pos] != b'{' {
            return Err(FonError::Parse(
                "Object array element must start with '{'".into(),
            ));
        }
        let (obj, consumed) = parse_object(&content[pos..], depth, opts, interner)?;
        result.push(obj);
        pos += consumed;
        if pos < content.len() {
            pos = after_separator(content, pos)?;
        }
    }

    Ok((result, close + 1))
}
