# FON (Rust)

Fast Object Notation — the high-performance serialization core, implemented as an
idiomatic Rust library. A fast, human-readable key-value alternative to JSON.

This crate is consumer-agnostic: it exposes a normal Rust API and knows nothing
about FFI. C-ABI bindings for other languages (for example .NET via `FON.net`)
wrap this crate in their own repositories.

Part of the [FastObjectNotation](https://github.com/FastObjectNotation) family.

## Usage

```rust
use fon::types::{FonCollection, FonValue};
use fon::{serialize_to_string, deserialize_line, DeserializeOptions};

let mut c = FonCollection::new();
c.add("id".into(), FonValue::Int(42));
c.add("name".into(), FonValue::String("Test".into()));

let text = serialize_to_string(&c);
let back = deserialize_line(text.as_bytes(), &DeserializeOptions::default()).unwrap();
```

## License

MIT
