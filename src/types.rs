use std::collections::HashMap;
use std::sync::Arc;

use crate::raw_data::RawData;


pub const TYPE_BYTE: u8 = b'e';
pub const TYPE_SHORT: u8 = b't';
pub const TYPE_INT: u8 = b'i';
pub const TYPE_UINT: u8 = b'u';
pub const TYPE_LONG: u8 = b'l';
pub const TYPE_ULONG: u8 = b'g';
pub const TYPE_FLOAT: u8 = b'f';
pub const TYPE_DOUBLE: u8 = b'd';
pub const TYPE_BOOL: u8 = b'b';
pub const TYPE_STRING: u8 = b's';
pub const TYPE_RAW: u8 = b'r';
pub const TYPE_OBJECT: u8 = b'o';


pub enum FonValue {
    Byte(u8),
    Short(i16),
    Int(i32),
    UInt(u32),
    Long(i64),
    ULong(u64),
    Float(f32),
    Double(f64),
    Bool(bool),
    String(String),
    Raw(Box<RawData>),
    Object(Box<FonCollection>),
    ByteArray(Vec<u8>),
    ShortArray(Vec<i16>),
    IntArray(Vec<i32>),
    UIntArray(Vec<u32>),
    LongArray(Vec<i64>),
    ULongArray(Vec<u64>),
    FloatArray(Vec<f32>),
    DoubleArray(Vec<f64>),
    BoolArray(Vec<bool>),
    StringArray(Vec<String>),
    #[allow(clippy::vec_box)] // Box<FonCollection> matches the public ObjectArray variant type
    ObjectArray(Vec<Box<FonCollection>>),
}


impl FonValue {
    pub fn type_char(&self) -> u8 {
        match self {
            FonValue::Byte(_) | FonValue::ByteArray(_) => TYPE_BYTE,
            FonValue::Short(_) | FonValue::ShortArray(_) => TYPE_SHORT,
            FonValue::Int(_) | FonValue::IntArray(_) => TYPE_INT,
            FonValue::UInt(_) | FonValue::UIntArray(_) => TYPE_UINT,
            FonValue::Long(_) | FonValue::LongArray(_) => TYPE_LONG,
            FonValue::ULong(_) | FonValue::ULongArray(_) => TYPE_ULONG,
            FonValue::Float(_) | FonValue::FloatArray(_) => TYPE_FLOAT,
            FonValue::Double(_) | FonValue::DoubleArray(_) => TYPE_DOUBLE,
            FonValue::Bool(_) | FonValue::BoolArray(_) => TYPE_BOOL,
            FonValue::String(_) | FonValue::StringArray(_) => TYPE_STRING,
            FonValue::Raw(_) => TYPE_RAW,
            FonValue::Object(_) | FonValue::ObjectArray(_) => TYPE_OBJECT,
        }
    }
}


// Records have a handful of fields, so a flat Vec beats a per-record HashMap:
// far fewer allocations on the parse hot path, and `get` is a short linear scan.
// Keys are `Arc<str>` so a repeated key (every record shares the same field
// names) is interned once and reused via cheap clones.
#[derive(Default)]
pub struct FonCollection {
    data: Vec<(Arc<str>, FonValue)>,
}


impl FonCollection {
    pub fn new() -> Self {
        Self::default()
    }


    pub fn add(&mut self, key: impl Into<Arc<str>>, value: FonValue) {
        self.data.push((key.into(), value));
    }


    pub fn get(&self, key: &str) -> Option<&FonValue> {
        self.data.iter().find(|(k, _)| k.as_ref() == key).map(|(_, v)| v)
    }


    pub fn get_mut(&mut self, key: &str) -> Option<&mut FonValue> {
        self.data.iter_mut().find(|(k, _)| k.as_ref() == key).map(|(_, v)| v)
    }


    pub fn len(&self) -> usize {
        self.data.len()
    }


    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }


    pub fn iter(&self) -> impl Iterator<Item = (&Arc<str>, &FonValue)> {
        self.data.iter().map(|(k, v)| (k, v))
    }
}


// Records keyed by id. `Dense` (ids are exactly 0..len) is the fast path used by
// deserialization — no per-record hashing. Any non-contiguous insert promotes to
// `Sparse`.
enum FonDumpStore {
    Dense(Vec<FonCollection>),
    Sparse(HashMap<u64, FonCollection>),
}


impl Default for FonDumpStore {
    fn default() -> Self {
        FonDumpStore::Sparse(HashMap::new())
    }
}


#[derive(Default)]
pub struct FonDump {
    store: FonDumpStore,
}


impl FonDump {
    pub fn new() -> Self {
        Self::default()
    }


    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            store: FonDumpStore::Sparse(HashMap::with_capacity(capacity)),
        }
    }


    // Build a dump from records keyed densely by position (0..len): the fast path
    // for deserialization, no hashing.
    pub fn from_records(records: Vec<FonCollection>) -> Self {
        Self {
            store: FonDumpStore::Dense(records),
        }
    }


    pub fn add(&mut self, id: u64, collection: FonCollection) {
        match &mut self.store {
            FonDumpStore::Sparse(m) => {
                m.insert(id, collection);
                return;
            }
            FonDumpStore::Dense(v) if id as usize == v.len() => {
                v.push(collection);
                return;
            }
            FonDumpStore::Dense(_) => {}
        }
        // Non-contiguous id into a dense store: promote to sparse.
        let old = std::mem::take(&mut self.store);
        let mut map: HashMap<u64, FonCollection> = match old {
            FonDumpStore::Dense(v) => v.into_iter().enumerate().map(|(i, c)| (i as u64, c)).collect(),
            FonDumpStore::Sparse(m) => m,
        };
        map.insert(id, collection);
        self.store = FonDumpStore::Sparse(map);
    }


    pub fn get(&self, id: u64) -> Option<&FonCollection> {
        match &self.store {
            FonDumpStore::Dense(v) => v.get(id as usize),
            FonDumpStore::Sparse(m) => m.get(&id),
        }
    }


    pub fn get_mut(&mut self, id: u64) -> Option<&mut FonCollection> {
        match &mut self.store {
            FonDumpStore::Dense(v) => v.get_mut(id as usize),
            FonDumpStore::Sparse(m) => m.get_mut(&id),
        }
    }


    pub fn len(&self) -> usize {
        match &self.store {
            FonDumpStore::Dense(v) => v.len(),
            FonDumpStore::Sparse(m) => m.len(),
        }
    }


    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }


    pub fn iter(&self) -> FonDumpIter<'_> {
        match &self.store {
            FonDumpStore::Dense(v) => FonDumpIter::Dense(v.iter().enumerate()),
            FonDumpStore::Sparse(m) => FonDumpIter::Sparse(m.iter()),
        }
    }
}


pub enum FonDumpIter<'a> {
    Dense(std::iter::Enumerate<std::slice::Iter<'a, FonCollection>>),
    Sparse(std::collections::hash_map::Iter<'a, u64, FonCollection>),
}


impl<'a> Iterator for FonDumpIter<'a> {
    type Item = (u64, &'a FonCollection);

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            FonDumpIter::Dense(it) => it.next().map(|(i, c)| (i as u64, c)),
            FonDumpIter::Sparse(it) => it.next().map(|(k, c)| (*k, c)),
        }
    }
}
