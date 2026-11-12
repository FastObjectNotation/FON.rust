# FON — Fast Object Notation

[![CI](https://github.com/FastObjectNotation/FON.rust/actions/workflows/ci.yml/badge.svg)](https://github.com/FastObjectNotation/FON.rust/actions/workflows/ci.yml)

A fast, human-readable, line-oriented serialization format — a compact
alternative to JSON for record-style data; each line is one record; values are
typed and can nest.

## Features

- **Compact, readable wire format** — `key=type:value` pairs, one record per line.
- **Typed values** — numbers, bool, strings, binary blobs, nested objects, and
  arrays of any of them.
- **Nested objects and arrays of objects**, with a configurable maximum depth.
- **Parallel** dump serialization and deserialization via [Rayon](https://crates.io/crates/rayon).
- **Byte-oriented parsing** — reads straight from `&[u8]`, tolerates a UTF-8 BOM.
- **Editable in memory** — add, replace and remove records and fields, then save.
- **Z85 encoding** for raw blobs (5 ASCII characters per 4 bytes).

## Format

Each line is one record: a comma-separated list of `key=type:value` pairs. A
`.fon` file is a sequence of records, indexed by line number (0-based). An empty
line is an empty record. A line ends with `\n`, `\r\n` or `\r`; the last line
needs no terminator.

```
name=s:"John",age=i:30,balance=d:1234.56
scores=i:[95,87,92],tags=s:["admin","user"]
user=o:{id=i:42,name=s:"Bob",addr=o:{city=s:"NY",zip=i:10001}}
items=o:[{id=i:1,qty=i:5},{id=i:2,qty=i:3}]
blob=r:"?MsJX"
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
| `r`  | `RawData` (Z85) | `data=r:"?MsJX"`              |
| `o`  | `FonCollection` | `user=o:{id=i:1}`             |

Every primitive and string type also has an array form (`xs=i:[1,2,3]`). `r`
has no array form. `o` supports both nested objects (`{...}`) and arrays of
objects (`[{...},{...}]`). Bool is written as `0` or `1`; floats may be `nan`,
`inf` and `-inf`.

Strings are double-quoted and may contain any character, including U+0000. The
escapes `\n \r \t \b \f \" \\` are used, and every other character below U+0020
is written as `\uXXXX` (for example `\u0000` or `\u001B`). Parsing decodes
`\uXXXX` as well and rejects any other escape. Keys are not escaped; see
[Editing records](#editing-records).

Parsing is strict. Text that does not follow the format is an error, never a
guess:

- An integer must be the whole value: `12abc`, a lone sign and an out-of-range
  number are errors. A float that overflows (`1e400`) is an error.
- A bool is exactly `0` or `1`.
- Pairs and array items are separated by exactly one comma, with no comma after
  the last one and no spaces around separators (whitespace is part of a key or
  a value).
- A key may appear only once in a record.
- A UTF-8 byte order mark is skipped at the very start of the input only.

## Install

```bash
cargo add FastObjectNotation
```

The crate is published as `FastObjectNotation` and imported as `fon`.

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

### Many records

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

// threads = 0 uses all cores (see "Binary data and parse options").
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
use fon::serialize_to_string;
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

println!("{}", serialize_to_string(&user));
// id=i:42,addr=o:{city=s:"NY",zip=i:10001},items=o:[{qty=i:5},{qty=i:3}]
```

### Editing records

A record is a `FonCollection`; a dump is a `FonDump` (id to record). Both can
be changed in place.

Collection methods:

- `add_field(key, value)` appends a field. It returns an error if the key
  already exists or contains a reserved character.
- `set(key, value)` overwrites an existing field, and the type may change. It
  returns an error if the key is absent.
- `remove_field(key)` removes a field and returns `true` if it existed.
- `add(key, value)` appends a field without checking the key. Prefer
  `add_field` for keys that did not come from your own code.

Keys are written without escaping, so a key may not contain `=`, `"`, `{`, `}`,
`[`, `]`, CR or LF, and may not start with `,`. `fon::types::validate_key`
performs this check; `add_field` and the parser apply it.

Dump methods:

- `add(id, record)` stores a record under `id`. A record already at that id is
  replaced.
- `replace(id, record)` replaces the record at `id`. It returns an error if
  `id` has no record.
- `remove(id)` returns `true` if a record was removed.
- `remove_many(&ids)` removes several records and returns how many existed.
  Absent and duplicate ids are not counted.
- `get_mut(id)` edits a record in place.

Ids are stable in memory: removing a record never changes the ids of the
others. Saving writes the remaining records in ascending id order with no gaps,
so the ids are renumbered from 0 when the file is loaded again. A removed id
keeps a small placeholder until the dump is dropped, so a long-lived dump that
keeps adding new ids and removing old ones grows; save and reload it to compact.

`FonCollection`, `FonValue` and `RawData` implement `Clone`, which makes a deep
copy. `FonDump` does not implement `Clone`.

```rust
use fon::types::{validate_key, FonCollection, FonDump, FonValue};

let mut record = FonCollection::new();
record.add_field("qty", FonValue::Int(1)).unwrap();
assert!(record.add_field("qty", FonValue::Int(2)).is_err()); // already exists
record.set("qty", FonValue::Int(5)).unwrap();
assert!(record.set("missing", FonValue::Int(0)).is_err());   // not a field yet
assert!(validate_key("a=b").is_err());                        // reserved character
let copy = record.clone();                                    // deep copy

let mut dump = FonDump::new();
for id in 0..4u64 {
    let mut r = FonCollection::new();
    r.add("n", FonValue::Int(id as i32));
    dump.add(id, r);
}
assert!(dump.remove(1));                        // true: record 1 existed
assert_eq!(dump.remove_many(&[0, 0, 7, 2]), 2); // 0 and 2 removed; duplicate 0 and absent 7 not counted

let mut r = FonCollection::new();
r.add("n", FonValue::Int(30));
dump.replace(3, r).unwrap();                    // errors if id 3 had no record
```

### Binary data and parse options

```rust
use fon::serialize_to_string;
use fon::types::{FonCollection, FonValue};
use fon::{deserialize_line, DeserializeOptions, RawData};

let mut record = FonCollection::new();
record.add("blob", FonValue::Raw(Box::new(RawData::from_bytes(vec![0xDE, 0xAD, 0xBE, 0xEF]))));
let line = serialize_to_string(&record);
// blob=r:"?MsJX"

// Defaults: max_depth = 64, unpack_raw = false.
// `max_depth` bounds {}/[] nesting (too deep -> FonError::Parse). Values above
// fon::MAX_NESTING_DEPTH (128) are treated as 128.
// `unpack_raw: false` keeps a blob as Z85 text (RawData::encoded());
// `unpack_raw: true` decodes it to bytes during parsing (RawData::data()).
let opts = DeserializeOptions { max_depth: 32, unpack_raw: true };
let parsed = deserialize_line(line.as_bytes(), &opts).unwrap();
```

A record built through the API can nest deeper than the parser accepts. Such a
record is written without complaint but cannot be read back, and a tree
thousands of levels deep can exhaust the stack when it is written, cloned or
dropped.

The `threads` argument of the serialize and deserialize functions: `0` or a
negative value uses all cores. A value greater than `0` caps the worker threads
for that call; it is never more than the number of cores the machine has.

## API

The value and collection types are in `fon::types` and are also re-exported
from the crate root.

Collection (`FonCollection`):

| Item | Purpose |
|------|---------|
| `FonCollection::new()` | An empty record. |
| `add(key, value)` | Append a field without checking the key. |
| `add_field(key, value) -> Result<(), FonError>` | Append a field; rejects an existing key or a reserved character. |
| `set(key, value) -> Result<(), FonError>` | Overwrite an existing field; error if absent. |
| `get(key) -> Option<&FonValue>` | Read a field. |
| `get_mut(key) -> Option<&mut FonValue>` | Edit a field in place. |
| `remove_field(key) -> bool` | Remove a field. |
| `len()`, `is_empty()` | Number of fields. |
| `iter()` | `(&Arc<str>, &FonValue)` pairs in insertion order. |

Dump (`FonDump`):

| Item | Purpose |
|------|---------|
| `FonDump::new()`, `with_capacity(n)` | An empty dump. |
| `from_records(Vec<FonCollection>)` | A dump with records at ids `0..n`. |
| `add(id, record)` | Store a record; replaces one already at `id`. |
| `replace(id, record) -> Result<(), FonError>` | Replace an existing record; error if absent. |
| `get(id) -> Option<&FonCollection>` | Read a record. |
| `get_mut(id) -> Option<&mut FonCollection>` | Edit a record in place. |
| `remove(id) -> bool` | Remove one record. |
| `remove_many(&[u64]) -> usize` | Remove several records; returns how many existed. |
| `len()`, `is_empty()` | Number of records. |
| `iter()` | `(u64, &FonCollection)` pairs; order is not guaranteed. |

Functions:

| Function | Purpose |
|----------|---------|
| `serialize_to_string(&FonCollection) -> String` | One record to a line. |
| `serialize_dump_to_string(&FonDump, threads) -> String` | A whole dump to text, one line per record. |
| `serialize_dump_lines(&FonDump, threads) -> Vec<String>` | A dump as one string per record, in ascending id order. |
| `serialize_to_file(&FonDump, &Path, threads) -> Result<(), FonError>` | A dump to a `.fon` file. |
| `deserialize_line(&[u8], &DeserializeOptions) -> Result<FonCollection, FonError>` | Parse one record. |
| `deserialize_dump_from_bytes(&[u8], threads, &DeserializeOptions) -> Result<FonDump, FonError>` | Parse a buffer of records. |
| `deserialize_from_file(&Path, threads, &DeserializeOptions) -> Result<FonDump, FonError>` | Parse a `.fon` file. |
| `fon::types::validate_key(&str) -> Result<(), FonError>` | Check a key for reserved characters. |

Other public types: `FonValue` (the value enum; `type_char()` returns its
code), `RawData` (`from_bytes`, `from_encoded`, `data`, `encoded`, `pack`,
`unpack`), `DeserializeOptions` (`max_depth`, `unpack_raw`), and `FonError`
with the variants `Parse`, `Write`, `InvalidArgument` and `Io` (a file that
could not be read), each carrying a message. A parse error from a dump starts
with the line number, for example `line 12: Failed to parse number: 'x'`.

## Build and test

```bash
cargo build --release
cargo clippy --all-targets -- -D warnings
cargo test
```
