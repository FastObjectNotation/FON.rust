//! Recursive-descent parsing of one record (a line) into a `FonCollection`,
//! including nested objects and object arrays.

use crate::error::FonError;
use crate::raw_data::RawData;
use crate::types::{
    FonCollection, FonValue, TYPE_BOOL, TYPE_BYTE, TYPE_DOUBLE, TYPE_FLOAT, TYPE_INT, TYPE_LONG,
    TYPE_OBJECT, TYPE_RAW, TYPE_SHORT, TYPE_STRING, TYPE_UINT, TYPE_ULONG,
};

use super::scalar::{parse_bool_array, parse_num, parse_number_array, parse_string, parse_string_array};
use super::scan::{find_byte, find_closing_brace, find_closing_bracket, find_value_end};
use super::{as_str, intern, DeserializeOptions, KeyInterner};


pub(super) fn parse_collection_body<'a>(
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

    let end = find_value_end(data);
    let num = &data[..end];
    let mut consumed = end;
    if consumed < data.len() && data[consumed] == b',' {
        consumed += 1;
    }

    let value = match type_char {
        TYPE_BYTE => FonValue::Byte(parse_num::<u8>(num)?),
        TYPE_SHORT => FonValue::Short(parse_num::<i16>(num)?),
        TYPE_INT => FonValue::Int(parse_num::<i32>(num)?),
        TYPE_UINT => FonValue::UInt(parse_num::<u32>(num)?),
        TYPE_LONG => FonValue::Long(parse_num::<i64>(num)?),
        TYPE_ULONG => FonValue::ULong(parse_num::<u64>(num)?),
        TYPE_FLOAT => FonValue::Float(parse_num::<f32>(num)?),
        TYPE_DOUBLE => FonValue::Double(parse_num::<f64>(num)?),
        TYPE_BOOL => FonValue::Bool(data[0] != b'0'),
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
