use fon::types::{FonCollection, FonDump, FonValue};
use fon::{deserialize_dump_from_bytes, serialize_dump_to_string, DeserializeOptions};


// Large enough to span several 1 MB parse chunks, so the thread cap really changes
// how the work is split.
fn sample(n: u64) -> FonDump {
    let mut d = FonDump::new();
    for i in 0..n {
        let mut c = FonCollection::new();
        c.add("id", FonValue::ULong(i));
        c.add("name", FonValue::String(format!("record \"number\" {i}\n")));
        c.add("tags", FonValue::StringArray(vec!["a, b".into(), "c\\".into()]));
        d.add(i, c);
    }
    d
}


#[test]
fn max_threads_1_and_2_match_default_output() {
    let dump = sample(50_000);
    let opts = DeserializeOptions::default();
    let expected = serialize_dump_to_string(&dump, 0);

    for threads in [-1, 1, 2] {
        assert_eq!(serialize_dump_to_string(&dump, threads), expected, "serialize max_threads={threads}");

        let back = deserialize_dump_from_bytes(expected.as_bytes(), threads, &opts).unwrap();
        assert_eq!(back.len(), 50_000, "deserialize max_threads={threads}");
        assert_eq!(serialize_dump_to_string(&back, 0), expected, "deserialize max_threads={threads}");
    }
}
