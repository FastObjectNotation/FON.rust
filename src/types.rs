use std::collections::HashMap;

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


#[derive(Default)]
pub struct FonCollection {
    data: HashMap<String, FonValue>,
}


impl FonCollection {
    pub fn new() -> Self {
        Self::default()
    }


    pub fn add(&mut self, key: String, value: FonValue) {
        self.data.insert(key, value);
    }


    pub fn get(&self, key: &str) -> Option<&FonValue> {
        self.data.get(key)
    }


    pub fn get_mut(&mut self, key: &str) -> Option<&mut FonValue> {
        self.data.get_mut(key)
    }


    pub fn len(&self) -> usize {
        self.data.len()
    }


    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }


    pub fn iter(&self) -> std::collections::hash_map::Iter<'_, String, FonValue> {
        self.data.iter()
    }
}


#[derive(Default)]
pub struct FonDump {
    data: HashMap<u64, FonCollection>,
}


impl FonDump {
    pub fn new() -> Self {
        Self::default()
    }


    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            data: HashMap::with_capacity(capacity),
        }
    }


    pub fn add(&mut self, id: u64, collection: FonCollection) {
        self.data.insert(id, collection);
    }


    pub fn get(&self, id: u64) -> Option<&FonCollection> {
        self.data.get(&id)
    }


    pub fn get_mut(&mut self, id: u64) -> Option<&mut FonCollection> {
        self.data.get_mut(&id)
    }


    pub fn len(&self) -> usize {
        self.data.len()
    }


    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }


    pub fn iter(&self) -> std::collections::hash_map::Iter<'_, u64, FonCollection> {
        self.data.iter()
    }
}
