//! FON (Fast Object Notation) — high-performance serialization core.
//!
//! Idiomatic Rust library: types, serialization, and deserialization for the
//! FON wire format. This crate is consumer-agnostic; FFI / C-ABI bindings live
//! in the consuming projects (for example FON.net).

pub mod deserialize;
pub mod error;
pub mod raw_data;
pub mod serialize;
pub mod types;


pub use deserialize::{
    deserialize_dump_from_bytes, deserialize_from_file, deserialize_line, DeserializeOptions,
};
pub use error::FonError;
pub use raw_data::RawData;
pub use serialize::{
    serialize_dump_lines, serialize_dump_to_string, serialize_to_file, serialize_to_string,
};
pub use types::{FonCollection, FonDump, FonValue};
