use fon::types::{FonCollection, FonValue};
use fon::{deserialize_dump_from_bytes, deserialize_line, serialize_to_string, DeserializeOptions};


fn roundtrip(c: &FonCollection) -> FonCollection {
    let s = serialize_to_string(c);
    deserialize_line(s.as_bytes(), &DeserializeOptions::default()).unwrap()
}


#[test]
fn primitives_roundtrip() {
    let mut c = FonCollection::new();
    c.add("id".into(), FonValue::Int(42));
    c.add("name".into(), FonValue::String("Test".into()));
    c.add("active".into(), FonValue::Bool(true));
    let back = roundtrip(&c);
    assert!(matches!(back.get("id"), Some(FonValue::Int(42))));
    assert!(matches!(back.get("active"), Some(FonValue::Bool(true))));
    match back.get("name") {
        Some(FonValue::String(s)) => assert_eq!(s.as_str(), "Test"),
        _ => panic!("expected string value for name"),
    }
}


#[test]
fn int_array_roundtrip() {
    let mut c = FonCollection::new();
    c.add("xs".into(), FonValue::IntArray(vec![1, 2, 3]));
    let back = roundtrip(&c);
    match back.get("xs") {
        Some(FonValue::IntArray(v)) => assert_eq!(v, &vec![1, 2, 3]),
        _ => panic!("expected int array"),
    }
}


#[test]
fn nested_object_roundtrip() {
    let mut addr = FonCollection::new();
    addr.add("zip".into(), FonValue::Int(10001));
    let mut c = FonCollection::new();
    c.add("addr".into(), FonValue::Object(Box::new(addr)));
    let back = roundtrip(&c);
    match back.get("addr") {
        Some(FonValue::Object(b)) => assert!(matches!(b.get("zip"), Some(FonValue::Int(10001)))),
        _ => panic!("expected nested object"),
    }
}


#[test]
fn max_depth_enforced() {
    let s = "a=o:{b=o:{c=i:1}}";
    let shallow = DeserializeOptions { max_depth: 1, unpack_raw: false };
    assert!(deserialize_line(s.as_bytes(), &shallow).is_err());
    assert!(deserialize_line(s.as_bytes(), &DeserializeOptions::default()).is_ok());
}


#[test]
fn bom_is_stripped_in_dump() {
    let mut c = FonCollection::new();
    c.add("id".into(), FonValue::Int(7));
    let line = serialize_to_string(&c);
    let mut bytes = vec![0xEF, 0xBB, 0xBF];
    bytes.extend_from_slice(line.as_bytes());
    let dump = deserialize_dump_from_bytes(&bytes, 0, &DeserializeOptions::default()).unwrap();
    let first = dump.get(0).expect("record 0");
    assert!(matches!(first.get("id"), Some(FonValue::Int(7))));
}
