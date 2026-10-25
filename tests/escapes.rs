use fon::types::{FonCollection, FonDump, FonValue};
use fon::{
    deserialize_dump_from_bytes, deserialize_line, serialize_dump_to_string, serialize_to_string,
    DeserializeOptions,
};


fn parse(line: &[u8]) -> FonCollection {
    deserialize_line(line, &DeserializeOptions::default()).unwrap()
}


fn roundtrip(c: &FonCollection) -> FonCollection {
    parse(serialize_to_string(c).as_bytes())
}


fn string_of<'a>(c: &'a FonCollection, key: &str) -> &'a str {
    match c.get(key) {
        Some(FonValue::String(s)) => s.as_str(),
        _ => panic!("expected string value for {key}"),
    }
}


#[test]
fn scalar_string_ending_with_backslash_parses() {
    let c = parse(br#"path=s:"C:\\dir\\""#);
    assert_eq!(string_of(&c, "path"), r"C:\dir\");
}


#[test]
fn string_ending_with_backslash_roundtrips() {
    let mut c = FonCollection::new();
    c.add("path", FonValue::String(r"C:\dir\".into()));
    let back = roundtrip(&c);
    assert_eq!(string_of(&back, "path"), r"C:\dir\");
}


#[test]
fn string_array_item_ending_with_backslash_parses() {
    let c = parse(br#"xs=s:["C:\\","D:\\","plain"]"#);
    match c.get("xs") {
        Some(FonValue::StringArray(v)) => assert_eq!(
            v,
            &vec![r"C:\".to_string(), r"D:\".to_string(), "plain".to_string()]
        ),
        _ => panic!("expected string array"),
    }
}


#[test]
fn nested_object_string_ending_with_backslash_parses() {
    let c = parse(br#"addr=o:{path=s:"C:\\dir\\",zip=i:10001}"#);
    match c.get("addr") {
        Some(FonValue::Object(o)) => {
            assert_eq!(string_of(o, "path"), r"C:\dir\");
            assert!(matches!(o.get("zip"), Some(FonValue::Int(10001))));
        }
        _ => panic!("expected nested object"),
    }
}


#[test]
fn object_array_string_ending_with_backslash_parses() {
    let c = parse(br#"rows=o:[{p=s:"C:\\"},{p=s:"D:\\"}]"#);
    match c.get("rows") {
        Some(FonValue::ObjectArray(rows)) => {
            assert_eq!(rows.len(), 2);
            assert_eq!(string_of(&rows[0], "p"), r"C:\");
            assert_eq!(string_of(&rows[1], "p"), r"D:\");
        }
        _ => panic!("expected object array"),
    }
}


#[test]
fn every_control_char_roundtrips() {
    for b in 0u8..0x20 {
        let original = char::from(b).to_string();
        let mut c = FonCollection::new();
        c.add("s", FonValue::String(original.clone()));
        let back = roundtrip(&c);
        assert_eq!(string_of(&back, "s"), original, "control char 0x{b:02X}");
    }

    let all: String = (0u8..0x20).map(char::from).collect();
    let mut c = FonCollection::new();
    c.add("s", FonValue::String(all.clone()));
    let back = roundtrip(&c);
    assert_eq!(string_of(&back, "s"), all);
}


#[test]
fn unicode_escape_decodes() {
    let c = parse(br#"s=s:"caf\u00e9""#);
    assert_eq!(string_of(&c, "s"), "caf\u{e9}");

    let upper = parse(br#"s=s:"\u00E9\u0041""#);
    assert_eq!(string_of(&upper, "s"), "\u{e9}A");
}


#[test]
fn malformed_unicode_escape_is_an_error() {
    let opts = DeserializeOptions::default();
    assert!(deserialize_line(br#"s=s:"\u12""#, &opts).is_err());
    assert!(deserialize_line(br#"s=s:"\u12zz""#, &opts).is_err());
    assert!(deserialize_line(br#"s=s:"\uD800""#, &opts).is_err());
}


#[test]
fn unterminated_top_level_string_is_an_error() {
    let opts = DeserializeOptions::default();
    assert!(deserialize_line(br#"s=s:"abc"#, &opts).is_err());
    assert!(deserialize_line(br#"s=s:"abc\"#, &opts).is_err());
    assert!(deserialize_line(br#"s=s:"abc\""#, &opts).is_err());
    assert!(deserialize_line(br#"xs=s:["a","b"#, &opts).is_err());
}


#[test]
fn hundred_thousand_byte_strings_roundtrip() {
    let plain = "a".repeat(100_000);
    let escaped = "x\"y\\z".repeat(20_000);
    assert_eq!(escaped.len(), 100_000);

    let mut rec = FonCollection::new();
    rec.add("plain", FonValue::String(plain.clone()));
    rec.add("escaped", FonValue::String(escaped.clone()));
    let mut dump = FonDump::new();
    dump.add(0, rec);

    let text = serialize_dump_to_string(&dump, 0);
    let back = deserialize_dump_from_bytes(text.as_bytes(), 0, &DeserializeOptions::default())
        .unwrap();
    let c = back.get(0).unwrap();
    assert_eq!(string_of(c, "plain"), plain);
    assert_eq!(string_of(c, "escaped"), escaped);
}


#[test]
fn delimiters_inside_strings_in_nested_objects_roundtrip() {
    let tricky = "x}y,\"]z[{=:";

    let mut inner = FonCollection::new();
    inner.add("a", FonValue::String(tricky.into()));
    inner.add("b", FonValue::String("\\".into()));
    inner.add("c", FonValue::StringArray(vec![tricky.into(), "}]".into()]));

    let mut item = FonCollection::new();
    item.add("t", FonValue::String("{\"}".into()));
    item.add("n", FonValue::Int(7));

    let mut c = FonCollection::new();
    c.add("obj", FonValue::Object(Box::new(inner.clone())));
    c.add("rows", FonValue::ObjectArray(vec![Box::new(item.clone()), Box::new(item)]));

    let back = roundtrip(&c);
    match back.get("obj") {
        Some(FonValue::Object(o)) => {
            assert_eq!(string_of(o, "a"), tricky);
            assert_eq!(string_of(o, "b"), "\\");
            match o.get("c") {
                Some(FonValue::StringArray(v)) => {
                    assert_eq!(v, &vec![tricky.to_string(), "}]".to_string()]);
                }
                _ => panic!("expected string array in nested object"),
            }
        }
        _ => panic!("expected nested object"),
    }
    match back.get("rows") {
        Some(FonValue::ObjectArray(rows)) => {
            assert_eq!(rows.len(), 2);
            for row in rows {
                assert_eq!(string_of(row, "t"), "{\"}");
                assert!(matches!(row.get("n"), Some(FonValue::Int(7))));
            }
        }
        _ => panic!("expected object array"),
    }
}


#[test]
fn delimiters_inside_raw_nested_text_parse() {
    let c = parse(br#"o=o:{a=s:"}{",b=i:2},xs=s:["]","[,"]"#);
    match c.get("o") {
        Some(FonValue::Object(o)) => {
            assert_eq!(string_of(o, "a"), "}{");
            assert!(matches!(o.get("b"), Some(FonValue::Int(2))));
        }
        _ => panic!("expected nested object"),
    }
    match c.get("xs") {
        Some(FonValue::StringArray(v)) => assert_eq!(v, &vec!["]".to_string(), "[,".to_string()]),
        _ => panic!("expected string array"),
    }
}
