# FON — Fast Object Notation

[![CI](https://github.com/FastObjectNotation/FON.rust/actions/workflows/ci.yml/badge.svg)](https://github.com/FastObjectNotation/FON.rust/actions/workflows/ci.yml)

A fast, human-readable, line-oriented serialization format — a compact
alternative to JSON for record-style data. Each line is one record; values are
typed and can nest.

## Features

- **Compact, readable wire format** — `key=type:value` pairs, one record per line.
- **Typed values** — numeric/bool/string primitives, binary blobs, nested
  objects, and arrays of any of them.
- **Nested objects & arrays of objects**, with a configurable maximum depth.
- **Parallel** dump serialization and deserialization via [Rayon](https://crates.io/crates/rayon).
- **Byte-oriented parsing** — reads straight from `&[u8]`, BOM tolerant.
- **Z85 binary encoding** for raw blobs (5 ASCII chars per 4 bytes).

## Format

Each line is one record: a comma-separated list of `key=type:value` pairs. A
`.fon` file is a sequence of records, indexed by line number (0-based).

```
name=s:"John",age=i:30,balance=d:1234.56
scores=i:[95,87,92],tags=s:["admin","user"]
user=o:{id=i:42,name=s:"Bob",addr=o:{city=s:"NY",zip=i:10001}}
items=o:[{id=i:1,qty=i:5},{id=i:2,qty=i:3}]
blob=r:"nm=QNzv..."
```

### Type codes

| Code | Rust type       | Example                       |
|------|-----------------|-------------------------------|
| `e`  | `u8`            | `count=e:255`                 |
| `t`  | `i16`           | `year=t:2024`                 |
| `i`  | `i32`           | `id=i:42`                     |
| `u`  | `u32`           | `flags=u:12345`               |
| `l`  | `i64`           | `ts=l:1700000000`             |
| `g`  | `u64`           | `big=g:18446744073709551615`  |
| `f`  | `f32`           | `ratio=f:3.14`                |
| `d`  | `f64`           | `pi=d:3.141592653589793`      |
| `s`  | `String`        | `name=s:"Hello"`              |
| `b`  | `bool`          | `active=b:1`                  |
| `r`  | `RawData` (Z85) | `data=r:"nm=QNzv"`            |
| `o`  | `FonCollection` | `user=o:{id=i:1}`             |

Every primitive and string type also has an array form (`xs=i:[1,2,3]`), and `o`
supports both nested objects (`{...}`) and arrays of objects (`[{...},{...}]`).
Strings are double-quoted with `\n \r \t \b \f \" \\` escapes.

## Install

```toml
[dependencies]
fon = { git = "https://github.com/FastObjectNotation/FON.rust" }
```

## Usage

### A single record

```rust
use fon::types::{FonCollection, FonValue};
use fon::{serialize_to_string, deserialize_line, DeserializeOptions};

let mut record = FonCollection::new();
record.add("id", FonValue::Int(42));
record.add("name", FonValue::String("Test Item".into()));
record.add("price", FonValue::Double(99.99));
record.add("tags", FonValue::StringArray(vec!["sale".into(), "new".into()]));

let line = serialize_to_string(&record);
// id=i:42,name=s:"Test Item",price=d:99.99,tags=s:["sale","new"]

let parsed = deserialize_line(line.as_bytes(), &DeserializeOptions::default()).unwrap();
assert!(matches!(parsed.get("id"), Some(FonValue::Int(42))));
```

### Many records to/from a file

```rust
use std::path::Path;
use fon::types::{FonCollection, FonDump, FonValue};
use fon::{serialize_to_file, deserialize_from_file, DeserializeOptions};

let mut dump = FonDump::new();
for id in 0..1000u64 {
    let mut r = FonCollection::new();
    r.add("id", FonValue::ULong(id));
    r.add("text", FonValue::String(format!("row {id}")));
    dump.add(id, r);
}

// The `threads` argument hints the Rayon pool; 0 uses the global pool.
serialize_to_file(&dump, Path::new("data.fon"), 0).unwrap();

let loaded = deserialize_from_file(Path::new("data.fon"), 0, &DeserializeOptions::default()).unwrap();
for (id, record) in loaded.iter() {
    if let Some(FonValue::String(text)) = record.get("text") {
        println!("{id}: {text}");
    }
}
```

### Nested objects and arrays of objects

```rust
use fon::types::{FonCollection, FonValue};

let mut addr = FonCollection::new();
addr.add("city", FonValue::String("NY".into()));
addr.add("zip", FonValue::Int(10001));

let mut user = FonCollection::new();
user.add("id", FonValue::Int(42));
user.add("addr", FonValue::Object(Box::new(addr)));

let mut first = FonCollection::new();
first.add("qty", FonValue::Int(5));
let mut second = FonCollection::new();
second.add("qty", FonValue::Int(3));
user.add("items", FonValue::ObjectArray(vec![Box::new(first), Box::new(second)]));
```

### Binary data and parse options

```rust
use fon::types::FonValue;
use fon::{DeserializeOptions, RawData};

let blob = FonValue::Raw(Box::new(RawData::from_bytes(vec![0xDE, 0xAD, 0xBE, 0xEF])));

// Defaults: max_depth = 64, unpack_raw = false.
// `max_depth` bounds {}/[] nesting (too deep -> FonError::Parse).
// `unpack_raw` decodes RawData blobs from Z85 to bytes during parsing.
let opts = DeserializeOptions { max_depth: 32, unpack_raw: true };
```

## API

| Function | Purpose |
|----------|---------|
| `serialize_to_string(&FonCollection) -> String` | One record to a line. |
| `serialize_dump_to_string(&FonDump, threads) -> String` | A whole dump to text. |
| `serialize_to_file(&FonDump, &Path, threads) -> Result<(), FonError>` | A dump to a `.fon` file. |
| `deserialize_line(&[u8], &DeserializeOptions) -> Result<FonCollection, FonError>` | Parse one record. |
| `deserialize_dump_from_bytes(&[u8], threads, &DeserializeOptions) -> Result<FonDump, FonError>` | Parse a buffer of records. |
| `deserialize_from_file(&Path, threads, &DeserializeOptions) -> Result<FonDump, FonError>` | Parse a `.fon` file. |

Core types: `FonValue` (the value enum), `FonCollection` (a key→value map),
`FonDump` (id→collection map), `RawData` (Z85 blob), `FonError`.

> Placing a `FonCollection` inside itself (directly or transitively) is not
> detected and will recurse until the stack overflows during serialization.

## Build and test

```bash
cargo build --release
cargo clippy --all-targets -- -D warnings
cargo test
```
