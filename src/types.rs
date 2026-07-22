use std::collections::HashMap;
use std::sync::Arc;

use crate::error::FonError;
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


// A record cell in a mutable store. `value == None` is a tombstone: a removed record
// or an unfilled gap. The record is boxed so growing the slot vector never moves a live
// record — borrowed pointers into a live record survive any add/remove.
struct Slot {
    record_gen: u32,
    tree_gen: u32,
    value: Option<Box<FonCollection>>,
}


impl Slot {
    fn occupied(c: FonCollection) -> Self {
        Slot { record_gen: 0, tree_gen: 0, value: Some(Box::new(c)) }
    }
}


// `Dense` is the deserialization fast path (ids are exactly `0..len`, no boxing, no
// hashing). The first mutation promotes it to `Slab`, whose slots carry the generation
// counters used to validate borrowed views. `Sparse` holds the same generation-bearing
// slots for arbitrary / very large ids. `live` is the number of occupied slots so
// `len()` stays O(1) despite tombstones.
enum FonDumpStore {
    Dense(Vec<FonCollection>),
    Slab { slots: Vec<Slot>, live: usize },
    Sparse { map: HashMap<u64, Slot>, live: usize },
}


impl Default for FonDumpStore {
    fn default() -> Self {
        FonDumpStore::Sparse { map: HashMap::new(), live: 0 }
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
            store: FonDumpStore::Sparse { map: HashMap::with_capacity(capacity), live: 0 },
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
            FonDumpStore::Dense(v) if id as usize == v.len() => {
                v.push(collection);
                return;
            }
            FonDumpStore::Dense(_) => {
                // Non-contiguous add into a dense store: promote to sparse (historic behavior).
                let old = std::mem::take(&mut self.store);
                let (mut map, mut live) = match old {
                    FonDumpStore::Dense(v) => {
                        let live = v.len();
                        let map: HashMap<u64, Slot> =
                            v.into_iter().enumerate().map(|(i, c)| (i as u64, Slot::occupied(c))).collect();
                        (map, live)
                    }
                    _ => unreachable!(),
                };
                sparse_insert(&mut map, &mut live, id, collection);
                self.store = FonDumpStore::Sparse { map, live };
                return;
            }
            FonDumpStore::Slab { slots, live } => {
                let idx = id as usize;
                if idx == slots.len() {
                    slots.push(Slot::occupied(collection));
                    *live += 1;
                    return;
                }
                if idx < slots.len() {
                    let slot = &mut slots[idx];
                    if slot.value.is_none() {
                        *live += 1;
                    } else {
                        slot.record_gen = slot.record_gen.wrapping_add(1);
                    }
                    slot.value = Some(Box::new(collection));
                    return;
                }
                // Gap beyond the end: promote to sparse rather than pad unboundedly.
                let old = std::mem::take(&mut self.store);
                if let FonDumpStore::Slab { slots, live } = old {
                    let mut map: HashMap<u64, Slot> =
                        slots.into_iter().enumerate().map(|(i, s)| (i as u64, s)).collect();
                    let mut live = live;
                    sparse_insert(&mut map, &mut live, id, collection);
                    self.store = FonDumpStore::Sparse { map, live };
                }
                return;
            }
            FonDumpStore::Sparse { map, live } => {
                sparse_insert(map, live, id, collection);
                return;
            }
        }
    }


    pub fn get(&self, id: u64) -> Option<&FonCollection> {
        match &self.store {
            FonDumpStore::Dense(v) => v.get(id as usize),
            FonDumpStore::Slab { slots, .. } => {
                slots.get(id as usize).and_then(|s| s.value.as_deref())
            }
            FonDumpStore::Sparse { map, .. } => {
                map.get(&id).and_then(|s| s.value.as_deref())
            }
        }
    }


    pub fn get_mut(&mut self, id: u64) -> Option<&mut FonCollection> {
        match &mut self.store {
            FonDumpStore::Dense(v) => v.get_mut(id as usize),
            FonDumpStore::Slab { slots, .. } => {
                slots.get_mut(id as usize).and_then(|s| s.value.as_deref_mut())
            }
            FonDumpStore::Sparse { map, .. } => {
                map.get_mut(&id).and_then(|s| s.value.as_deref_mut())
            }
        }
    }


    pub fn len(&self) -> usize {
        match &self.store {
            FonDumpStore::Dense(v) => v.len(),
            FonDumpStore::Slab { live, .. } => *live,
            FonDumpStore::Sparse { live, .. } => *live,
        }
    }


    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }


    /// Promote a `Dense` store to `Slab` so records live in generation-bearing slots.
    /// No-op if already mutable. Call this before handing out an editable record pointer
    /// or before any field mutation, so a later promotion cannot move a record out from
    /// under a borrowed pointer.
    pub fn ensure_mutable(&mut self) {
        if let FonDumpStore::Dense(_) = &self.store {
            let old = std::mem::replace(&mut self.store, FonDumpStore::Slab { slots: Vec::new(), live: 0 });
            if let FonDumpStore::Dense(v) = old {
                let live = v.len();
                let slots = v.into_iter().map(Slot::occupied).collect();
                self.store = FonDumpStore::Slab { slots, live };
            }
        }
    }


    pub fn iter(&self) -> FonDumpIter<'_> {
        match &self.store {
            FonDumpStore::Dense(v) => FonDumpIter::Dense(v.iter().enumerate()),
            FonDumpStore::Slab { slots, .. } => FonDumpIter::Slab(slots.iter().enumerate()),
            FonDumpStore::Sparse { map, .. } => FonDumpIter::Sparse(map.iter()),
        }
    }


    /// Remove the record with `id`. Returns `true` if a live record was removed, `false`
    /// if there was none (not an error). Other records keep their ids — removal never
    /// shifts neighbours.
    pub fn remove(&mut self, id: u64) -> bool {
        self.ensure_mutable();
        match &mut self.store {
            FonDumpStore::Slab { slots, live } => {
                match slots.get_mut(id as usize) {
                    Some(slot) if slot.value.is_some() => {
                        slot.value = None;
                        slot.record_gen = slot.record_gen.wrapping_add(1);
                        *live -= 1;
                        true
                    }
                    _ => false,
                }
            }
            FonDumpStore::Sparse { map, live } => {
                match map.get_mut(&id) {
                    Some(slot) if slot.value.is_some() => {
                        slot.value = None;
                        slot.record_gen = slot.record_gen.wrapping_add(1);
                        *live -= 1;
                        true
                    }
                    _ => false,
                }
            }
            FonDumpStore::Dense(_) => unreachable!("ensure_mutable promoted away Dense"),
        }
    }


    /// Remove every id in `ids`, returning how many live records were actually removed
    /// (absent or duplicate ids don't count).
    pub fn remove_many(&mut self, ids: &[u64]) -> usize {
        self.ensure_mutable();
        let mut removed = 0;
        for &id in ids {
            if self.remove(id) {
                removed += 1;
            }
        }
        removed
    }


    /// Replace the record at `id` with a new collection. Errors if `id` has no live record
    /// (use `add` to create). Bumps the record's generation so borrowed views of the old
    /// record become stale.
    pub fn replace(&mut self, id: u64, collection: FonCollection) -> Result<(), FonError> {
        self.ensure_mutable();
        let slot = match &mut self.store {
            FonDumpStore::Slab { slots, .. } => slots.get_mut(id as usize),
            FonDumpStore::Sparse { map, .. } => map.get_mut(&id),
            FonDumpStore::Dense(_) => unreachable!("ensure_mutable promoted away Dense"),
        };
        match slot {
            Some(slot) if slot.value.is_some() => {
                slot.value = Some(Box::new(collection));
                slot.record_gen = slot.record_gen.wrapping_add(1);
                Ok(())
            }
            _ => Err(FonError::InvalidArgument(format!("no record with id {id} to replace"))),
        }
    }
}


// Insert/overwrite a sparse slot, keeping `live` accurate and bumping `record_gen`
// when an existing live record is overwritten or a tombstone is refilled.
fn sparse_insert(map: &mut HashMap<u64, Slot>, live: &mut usize, id: u64, c: FonCollection) {
    match map.get_mut(&id) {
        Some(slot) => {
            if slot.value.is_none() {
                *live += 1;
            }
            slot.record_gen = slot.record_gen.wrapping_add(1);
            slot.value = Some(Box::new(c));
        }
        None => {
            map.insert(id, Slot::occupied(c));
            *live += 1;
        }
    }
}


pub enum FonDumpIter<'a> {
    Dense(std::iter::Enumerate<std::slice::Iter<'a, FonCollection>>),
    Slab(std::iter::Enumerate<std::slice::Iter<'a, Slot>>),
    Sparse(std::collections::hash_map::Iter<'a, u64, Slot>),
}


impl<'a> Iterator for FonDumpIter<'a> {
    type Item = (u64, &'a FonCollection);

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            FonDumpIter::Dense(it) => it.next().map(|(i, c)| (i as u64, c)),
            FonDumpIter::Slab(it) => {
                for (i, slot) in it.by_ref() {
                    if let Some(c) = slot.value.as_deref() {
                        return Some((i as u64, c));
                    }
                }
                None
            }
            FonDumpIter::Sparse(it) => {
                for (k, slot) in it.by_ref() {
                    if let Some(c) = slot.value.as_deref() {
                        return Some((*k, c));
                    }
                }
                None
            }
        }
    }
}
