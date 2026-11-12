use fon::types::{validate_key, FonCollection, FonDump, FonValue};
use fon::{
    deserialize_dump_from_bytes, deserialize_from_file, deserialize_line, serialize_dump_lines,
    serialize_dump_to_string, serialize_to_file, serialize_to_string, DeserializeOptions, FonError,
    RawData, MAX_NESTING_DEPTH,
};


fn parse(line: &[u8]) -> Result<FonCollection, FonError> {
    deserialize_line(line, &DeserializeOptions::default())
}


fn parse_dump(text: &[u8], threads: i32) -> Result<FonDump, FonError> {
    deserialize_dump_from_bytes(text, threads, &DeserializeOptions::default())
}


fn rejected(line: &[u8]) {
    assert!(parse(line).is_err(), "{:?} accepted", String::from_utf8_lossy(line));
}


fn int_of(c: &FonCollection, key: &str) -> i32 {
    match c.get(key) {
        Some(FonValue::Int(v)) => *v,
        _ => panic!("no int field {key}"),
    }
}


fn message(result: Result<FonDump, FonError>) -> String {
    match result {
        Err(e) => e.to_string(),
        Ok(_) => panic!("expected an error"),
    }
}


#[test]
fn bool_arrays_with_stray_brackets_or_line_breaks_are_errors_not_hangs() {
    for line in [&b"a=b:[[]]"[..], b"a=b:[1,[2]]", b"a=b:[1\n]", b"a=b:[1\r]", b"a=b:[]]"] {
        rejected(line);
    }
    let text: &[u8] = b"x=i:1\na=b:[[]]\n";
    for threads in [1, 2, 0] {
        assert!(parse_dump(text, threads).is_err());
    }
}


#[test]
fn integers_must_be_the_whole_token() {
    for line in [
        &b"a=i:12abc"[..], b"a=i:12 34", b"a=u:5.5", b"a=t:-", b"a=i:+", b"a=i:0x10", b"a=i: 1",
        b"a=e:256", b"a=u:-1", b"a=l:9223372036854775808", b"a=i:[1,2x]", b"a=i:[1 2]",
    ] {
        rejected(line);
    }
    assert_eq!(int_of(&parse(b"a=i:+5").unwrap(), "a"), 5);
    assert_eq!(int_of(&parse(b"a=i:-7").unwrap(), "a"), -7);
    assert_eq!(int_of(&parse(b"a=i:007").unwrap(), "a"), 7);
}


#[test]
fn bool_is_exactly_zero_or_one() {
    for line in [
        &b"a=b:false"[..], b"a=b:true", b"a=b:x", b"a=b:2", b"a=b:1abc", b"a=b:10", b"a=b:[x y]",
        b"a=b:[1,,0]", b"a=b:[2]", b"a=b:[1,]",
    ] {
        rejected(line);
    }
    let c = parse(b"t=b:1,f=b:0,list=b:[1,0,1],none=b:[]").unwrap();
    assert!(matches!(c.get("t"), Some(FonValue::Bool(true))));
    assert!(matches!(c.get("f"), Some(FonValue::Bool(false))));
    assert_eq!(c.get("list"), Some(&FonValue::BoolArray(vec![true, false, true])));
    assert_eq!(c.get("none"), Some(&FonValue::BoolArray(vec![])));
}


#[test]
fn floats_reject_overflow_but_accept_the_written_special_values() {
    rejected(b"a=d:1e400");
    rejected(b"a=f:1e40");
    rejected(b"a=d:-1e400");
    rejected(b"a=d:1.5x");
    let c = parse(b"a=d:inf,b=d:-inf,c=d:nan,d=f:inf,e=d:1e308").unwrap();
    assert!(matches!(c.get("a"), Some(FonValue::Double(v)) if *v == f64::INFINITY));
    assert!(matches!(c.get("b"), Some(FonValue::Double(v)) if *v == f64::NEG_INFINITY));
    assert!(matches!(c.get("c"), Some(FonValue::Double(v)) if v.is_nan()));
    assert!(matches!(c.get("d"), Some(FonValue::Float(v)) if *v == f32::INFINITY));
    assert!(matches!(c.get("e"), Some(FonValue::Double(v)) if *v == 1e308));
}


#[test]
fn float_special_values_and_extremes_round_trip() {
    let values: [f64; 9] = [
        0.0, -0.0, f64::MAX, f64::MIN, f64::MIN_POSITIVE, 5e-324, f64::INFINITY, f64::NEG_INFINITY,
        0.1 + 0.2,
    ];
    let mut c = FonCollection::new();
    c.add("list", FonValue::DoubleArray(values.to_vec()));
    c.add("nan", FonValue::Double(f64::NAN));
    c.add("small", FonValue::FloatArray(vec![f32::MAX, f32::MIN_POSITIVE, -0.0, 16777217.0]));
    let back = parse(serialize_to_string(&c).as_bytes()).unwrap();
    match back.get("list") {
        Some(FonValue::DoubleArray(list)) => {
            let bits: Vec<u64> = list.iter().map(|v| v.to_bits()).collect();
            let expected: Vec<u64> = values.iter().map(|v| v.to_bits()).collect();
            assert_eq!(bits, expected);
        }
        _ => panic!("expected double array"),
    }
    assert!(matches!(back.get("nan"), Some(FonValue::Double(v)) if v.is_nan()));
    assert_eq!(back.get("small"), c.get("small"));
}


#[test]
fn structural_junk_is_an_error() {
    for line in [
        &b"hello world"[..],
        b"a=i:1,junk",
        b"a=s:\"x\"junk=i:1",
        b"a=s:\"x\"garbage,b=i:1",
        b"a=i:5]b=i:3",
        b"a=o:{a=i:1}x",
        b"a=i:1,",
        b"a=i:1,,b=i:2",
        b",a=i:1",
        b"a=s:[\"a\"\"b\"]",
        b"a=s:[\"a\",]",
        b"a=o:[{x=i:1}{y=i:2}]",
        b"a=o:[{x=i:1},]",
        b"a=i:[1,2,]",
        b"a=i:[1,,2]",
        b"a=i:[,1]",
        b"{a=i:1",
        b"a=i",
        b"a=i:",
        b"a=z:1",
    ] {
        rejected(line);
    }
}


#[test]
fn duplicate_keys_are_rejected_in_narrow_wide_and_nested_records() {
    rejected(b"a=i:1,a=i:2");
    rejected(b"o=o:{x=i:1,x=i:2}");
    // The same key in a parent and in its child is not a duplicate.
    assert!(parse(b"x=i:1,o=o:{x=i:2}").is_ok());

    let mut wide: String = (0..5000).map(|i| format!("k{i}=i:{i},")).collect();
    wide.push_str("last=i:1");
    let c = parse(wide.as_bytes()).unwrap();
    assert_eq!(c.len(), 5001);
    assert_eq!(int_of(&c, "k4999"), 4999);

    wide.push_str(",k4000=i:0");
    rejected(wide.as_bytes());
}


#[test]
fn unknown_escapes_are_rejected() {
    rejected(br#"a=s:"\q""#);
    rejected(br#"a=s:"\/""#);
    rejected(br#"a=s:["ok","\x41"]"#);
    let c = parse(br#"a=s:"\"\\\n\r\t\b\f\u0041""#).unwrap();
    assert_eq!(c.get("a"), Some(&FonValue::String("\"\\\n\r\t\u{8}\u{c}A".into())));
}


#[test]
fn a_single_record_may_end_with_one_line_terminator_only() {
    for line in [&b"a=i:1\n"[..], b"a=i:1\r\n", b"a=i:1\r", b"a=i:1"] {
        assert_eq!(int_of(&parse(line).unwrap(), "a"), 1);
    }
    rejected(b"a=i:1\nb=i:2");
    rejected(b"a=i:1\rb=i:2");
    rejected(b"a=i:1\n\n");
    rejected(b"a=s:\"x\ny\"");
}


#[test]
fn a_byte_order_mark_is_skipped_for_a_record_and_for_a_dump() {
    let c = parse(b"\xEF\xBB\xBFa=i:1").unwrap();
    assert_eq!(int_of(&c, "a"), 1);
    let dump = parse_dump(b"\xEF\xBB\xBFa=i:1\n", 1).unwrap();
    assert_eq!(int_of(dump.get(0).unwrap(), "a"), 1);
}


#[test]
fn an_empty_line_is_an_empty_record_and_ids_are_line_indexes() {
    let dump = parse_dump(b"a=i:1\n\nb=i:2", 1).unwrap();
    assert_eq!(dump.len(), 3);
    assert_eq!(int_of(dump.get(0).unwrap(), "a"), 1);
    assert!(dump.get(1).unwrap().is_empty());
    assert_eq!(int_of(dump.get(2).unwrap(), "b"), 2);

    assert_eq!(parse_dump(b"", 1).unwrap().len(), 0);
    assert_eq!(parse_dump(b"\n", 1).unwrap().len(), 1);
    assert_eq!(parse_dump(b"a=i:1\n", 1).unwrap().len(), 1);
    assert_eq!(parse_dump(b"a=i:1\r\n\r\n", 1).unwrap().len(), 2);
    assert_eq!(parse_dump(b"a=i:1\r\rb=i:2\r", 1).unwrap().len(), 3);
}


#[test]
fn empty_records_survive_a_save_and_load() {
    let mut dump = FonDump::new();
    let mut first = FonCollection::new();
    first.add("x", FonValue::Int(1));
    let mut third = FonCollection::new();
    third.add("x", FonValue::Int(3));
    dump.add(0, first);
    dump.add(1, FonCollection::new());
    dump.add(2, third);

    for threads in [1, 2, 0] {
        let text = serialize_dump_to_string(&dump, threads);
        assert_eq!(text, "x=i:1\n\nx=i:3\n");
        let back = parse_dump(text.as_bytes(), threads).unwrap();
        assert_eq!(back.len(), 3);
        assert!(back.get(1).unwrap().is_empty());
        assert_eq!(int_of(back.get(2).unwrap(), "x"), 3);
    }

    let mut only = FonDump::new();
    only.add(0, FonCollection::new());
    let text = serialize_dump_to_string(&only, 1);
    assert_eq!(parse_dump(text.as_bytes(), 1).unwrap().len(), 1);
}


#[test]
fn raw_text_with_syntax_characters_keeps_the_record_parseable() {
    let text = "ab\"cd\\ef\nline";
    let mut c = FonCollection::new();
    c.add("k", FonValue::Raw(Box::new(RawData::from_encoded(text.into()))));
    c.add("after", FonValue::Int(7));
    let line = serialize_to_string(&c);
    assert!(!line.contains('\n'));
    let back = parse(line.as_bytes()).unwrap();
    match back.get("k") {
        Some(FonValue::Raw(raw)) => assert_eq!(raw.encoded(), text),
        _ => panic!("expected raw value"),
    }
    assert_eq!(int_of(&back, "after"), 7);

    // Text that arrived escaped in the input is written escaped again.
    let parsed = parse(br#"k=r:"a\nb",z=i:2"#).unwrap();
    assert_eq!(serialize_to_string(&parsed), r#"k=r:"a\nb",z=i:2"#);
}


#[test]
fn dump_errors_name_the_first_bad_line_for_every_thread_count() {
    let mut text = String::new();
    for i in 0..4000 {
        text.push_str(&format!("id=i:{i},name=s:\"row {i}\"\n"));
    }
    let bad_line = 2501;
    let mut lines: Vec<&str> = text.lines().collect();
    lines[bad_line - 1] = "id=i:oops";
    lines[3200] = "id=i:also-bad";
    let joined = lines.join("\n");

    let expected = format!("line {bad_line}: Failed to parse number: 'oops'");
    for threads in [1, 2, 3, 8, 0] {
        assert_eq!(message(parse_dump(joined.as_bytes(), threads)), expected, "threads {threads}");
    }

    let crlf = lines.join("\r\n");
    assert_eq!(message(parse_dump(crlf.as_bytes(), 0)), expected);
}


#[test]
fn a_syntax_error_before_invalid_utf8_is_the_one_reported() {
    let syntax_first: &[u8] = b"a=i:1\nb=i:x\nc=s:\"\xFF\"\n";
    let utf8_first: &[u8] = b"a=i:1\nc=s:\"\xFF\"\nb=i:x\n";
    for threads in [1, 2, 8, 0] {
        assert_eq!(
            message(parse_dump(syntax_first, threads)),
            "line 2: Failed to parse number: 'x'",
            "threads {threads}"
        );
        assert_eq!(
            message(parse_dump(utf8_first, threads)),
            "line 2: Invalid UTF-8 in input",
            "threads {threads}"
        );
    }
    assert!(parse(b"c=s:\"\xFF\"").is_err());
}


#[test]
fn line_ends_of_every_kind_give_the_same_records_for_every_thread_count() {
    let count: usize = 6000;
    for end in ["\n", "\r\n", "\r"] {
        let text: String = (0..count).map(|i| format!("id=i:{i}{end}")).collect();
        for threads in [1, 3, 0] {
            let dump = parse_dump(text.as_bytes(), threads).unwrap();
            assert_eq!(dump.len(), count, "end {end:?}, threads {threads}");
            for id in [0usize, 1, 2999, count - 1] {
                assert_eq!(int_of(dump.get(id as u64).unwrap(), "id"), id as i32);
            }
        }
    }
}


#[test]
fn nesting_is_capped_whatever_max_depth_says() {
    let nested = |levels: usize| -> String {
        let mut text = String::new();
        for _ in 0..levels {
            text.push_str("a=o:{");
        }
        text.push_str("a=i:1");
        for _ in 0..levels {
            text.push('}');
        }
        text
    };
    let opts = DeserializeOptions { max_depth: i32::MAX, unpack_raw: false };
    let limit = MAX_NESTING_DEPTH as usize;

    let too_deep = nested(limit + 1);
    assert!(deserialize_line(too_deep.as_bytes(), &opts).is_err());

    // The deepest accepted input must fit a 1 MiB stack, the Windows main-thread default.
    let deepest = nested(limit);
    let handle = std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(move || {
            let opts = DeserializeOptions { max_depth: i32::MAX, unpack_raw: false };
            let c = deserialize_line(deepest.as_bytes(), &opts).unwrap();
            let again = serialize_to_string(&c);
            assert_eq!(again, deepest);
            let copy = c.clone();
            drop(c);
            copy.len()
        })
        .unwrap();
    assert_eq!(handle.join().unwrap(), 1);
}


#[test]
fn keys_may_not_start_with_a_comma() {
    assert!(validate_key(",a").is_err());
    let mut c = FonCollection::new();
    assert!(c.add_field(",a", FonValue::Int(1)).is_err());
    c.add_field("a,b", FonValue::Int(1)).unwrap();
    c.add_field("", FonValue::Int(2)).unwrap();
    c.add_field(" spaced key ", FonValue::Int(3)).unwrap();
    let back = parse(serialize_to_string(&c).as_bytes()).unwrap();
    assert_eq!(back, c);
}


#[test]
fn reserved_characters_in_parsed_keys_are_errors() {
    for line in [&b"a]b=i:1"[..], b"a[b=i:1", b"a{b=i:1", b"a}b=i:1", b"a\"b=i:1"] {
        rejected(line);
    }
}


#[test]
fn serialized_lines_are_sized_to_their_text() {
    let mut dump = FonDump::new();
    for id in 0..20_000u64 {
        let mut c = FonCollection::new();
        c.add("id", FonValue::ULong(id));
        dump.add(id, c);
    }
    let lines = serialize_dump_lines(&dump, 0);
    assert_eq!(lines.len(), 20_000);
    assert_eq!(lines[19_999], "id=g:19999");
    let text: usize = lines.iter().map(String::len).sum();
    let reserved: usize = lines.iter().map(String::capacity).sum();
    assert!(reserved <= text * 2, "{reserved} bytes reserved for {text} bytes of text");

    let joined = serialize_dump_to_string(&dump, 0);
    assert_eq!(joined.len(), text + lines.len());
    assert_eq!(joined, lines.join("\n") + "\n");
}


#[test]
fn a_sparse_dump_is_written_in_ascending_id_order() {
    let mut dump = FonDump::new();
    for id in [900u64, 5, 70, 1_000_000, 0] {
        let mut c = FonCollection::new();
        c.add("id", FonValue::ULong(id));
        dump.add(id, c);
    }
    for threads in [1, 0] {
        assert_eq!(
            serialize_dump_to_string(&dump, threads),
            "id=g:0\nid=g:5\nid=g:70\nid=g:900\nid=g:1000000\n"
        );
    }
}


#[test]
fn a_dump_round_trips_through_a_file() {
    let mut dump = FonDump::new();
    for id in 0..300u64 {
        let mut c = FonCollection::new();
        c.add("id", FonValue::ULong(id));
        c.add("text", FonValue::String(format!("row {id} \"quoted\"\n")));
        dump.add(id, c);
    }
    let path = std::env::temp_dir().join(format!("fon_strict_{}.fon", std::process::id()));
    serialize_to_file(&dump, &path, 0).unwrap();
    let back = deserialize_from_file(&path, 0, &DeserializeOptions::default());
    let _ = std::fs::remove_file(&path);
    let back = back.unwrap();
    assert_eq!(back.len(), 300);
    for id in [0u64, 150, 299] {
        assert_eq!(back.get(id), dump.get(id));
    }
}


#[test]
fn a_missing_file_is_an_io_error_and_a_missing_directory_a_write_error() {
    let missing = std::env::temp_dir().join(format!("fon_missing_{}", std::process::id())).join("x.fon");
    assert!(matches!(
        deserialize_from_file(&missing, 0, &DeserializeOptions::default()),
        Err(FonError::Io(_))
    ));
    assert!(matches!(serialize_to_file(&FonDump::new(), &missing, 0), Err(FonError::Write(_))));
}
