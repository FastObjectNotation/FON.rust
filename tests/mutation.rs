use fon::types::{FonCollection, FonDump, FonValue};

fn record(id: i32) -> FonCollection {
    let mut c = FonCollection::new();
    c.add("id", FonValue::Int(id));
    c
}

fn id_of(c: &FonCollection) -> i32 {
    match c.get("id") {
        Some(FonValue::Int(v)) => *v,
        _ => panic!("record has no int id"),
    }
}

#[test]
fn ensure_mutable_preserves_records_and_ids() {
    // Dense dump from contiguous records (the deserialization shape).
    let mut dump = FonDump::from_records(vec![record(10), record(11), record(12)]);
    assert_eq!(dump.len(), 3);

    dump.ensure_mutable(); // Dense -> Slab

    assert_eq!(dump.len(), 3);
    assert_eq!(id_of(dump.get(0).unwrap()), 10);
    assert_eq!(id_of(dump.get(1).unwrap()), 11);
    assert_eq!(id_of(dump.get(2).unwrap()), 12);
    assert!(dump.get(3).is_none());

    // ensure_mutable is idempotent.
    dump.ensure_mutable();
    assert_eq!(dump.len(), 3);
    assert_eq!(id_of(dump.get(2).unwrap()), 12);
}


// Dense, Slab with tombstones, and Sparse dumps, each with ids stored in the record
// so `nth` results can be checked against the plain `next` walk.
fn store_variants() -> Vec<(&'static str, FonDump)> {
    let dense = FonDump::from_records((0..10).map(record).collect());

    let mut slab = FonDump::from_records((0..10).map(record).collect());
    slab.ensure_mutable();
    slab.remove(2);
    slab.remove(5);
    slab.remove(9);

    let mut sparse = FonDump::new();
    sparse.add(5, record(5));
    sparse.add(0, record(0));
    sparse.add(10, record(10));
    sparse.add(11, record(11));
    sparse.add(100, record(100));
    sparse.remove(10);

    vec![("dense", dense), ("slab", slab), ("sparse", sparse)]
}


#[test]
fn nth_matches_next_walk_on_every_store() {
    for (name, dump) in store_variants() {
        let walked: Vec<(u64, i32)> = dump.iter().map(|(id, c)| (id, id_of(c))).collect();
        for k in 0..=walked.len() + 1 {
            let got = dump.iter().nth(k).map(|(id, c)| (id, id_of(c)));
            assert_eq!(got, walked.get(k).copied(), "store {name}, nth({k})");
        }
    }
}


#[test]
fn nth_then_next_continues_the_walk() {
    for (name, dump) in store_variants() {
        let walked: Vec<u64> = dump.iter().map(|(id, _)| id).collect();
        let mut it = dump.iter();
        let mut seen = Vec::new();
        if let Some((id, _)) = it.nth(1) {
            seen.push(id);
        }
        for (id, _) in it {
            seen.push(id);
        }
        assert_eq!(seen, walked[1..].to_vec(), "store {name}");
    }
}


#[test]
fn size_hint_is_exact_for_dense_and_a_valid_bound_otherwise() {
    let dense = FonDump::from_records((0..10).map(record).collect());
    assert_eq!(dense.iter().size_hint(), (10, Some(10)));

    for (name, dump) in store_variants() {
        let live = dump.iter().count();
        let (lower, upper) = dump.iter().size_hint();
        assert!(lower <= live, "store {name}");
        assert!(upper.is_none_or(|u| u >= live), "store {name}");
    }
}


#[test]
fn clone_is_deep() {
    let mut inner = FonCollection::new();
    inner.add("x", FonValue::Int(1));

    let mut original = FonCollection::new();
    original.add("n", FonValue::Int(1));
    original.add("inner", FonValue::Object(Box::new(inner)));
    original.add("tags", FonValue::StringArray(vec!["a".into()]));

    let mut copy = original.clone();
    copy.set("n", FonValue::Int(2)).unwrap();
    if let Some(FonValue::Object(o)) = copy.get_mut("inner") {
        o.set("x", FonValue::Int(9)).unwrap();
    }
    if let Some(FonValue::StringArray(v)) = copy.get_mut("tags") {
        v.push("b".into());
    }

    assert!(matches!(original.get("n"), Some(FonValue::Int(1))));
    match original.get("inner") {
        Some(FonValue::Object(o)) => assert!(matches!(o.get("x"), Some(FonValue::Int(1)))),
        _ => panic!("expected nested object"),
    }
    match original.get("tags") {
        Some(FonValue::StringArray(v)) => assert_eq!(v.len(), 1),
        _ => panic!("expected string array"),
    }
}

#[test]
fn add_after_promotion_appends_contiguously() {
    let mut dump = FonDump::from_records(vec![record(0), record(1)]);
    dump.ensure_mutable();
    dump.add(2, record(2)); // id == len -> stays Slab, appends
    assert_eq!(dump.len(), 3);
    assert_eq!(id_of(dump.get(2).unwrap()), 2);
}

#[test]
fn iter_after_promotion_yields_all_live_records() {
    let mut dump = FonDump::from_records(vec![record(100), record(101)]);
    dump.ensure_mutable();
    let mut ids: Vec<(u64, i32)> = dump.iter().map(|(id, c)| (id, id_of(c))).collect();
    ids.sort();
    assert_eq!(ids, vec![(0, 100), (1, 101)]);
}


#[test]
fn remove_keeps_other_ids_stable() {
    let mut dump = FonDump::from_records(vec![record(0), record(1), record(2), record(3)]);

    assert!(dump.remove(1));
    assert_eq!(dump.len(), 3);
    assert!(dump.get(1).is_none());          // tombstoned
    assert_eq!(id_of(dump.get(0).unwrap()), 0); // neighbours unmoved
    assert_eq!(id_of(dump.get(2).unwrap()), 2);
    assert_eq!(id_of(dump.get(3).unwrap()), 3);

    // Iteration (what serialization consumes) skips the tombstone, so a save/load
    // renumbers the survivors 0,1,2 — the documented id-across-serialize behavior.
    let mut ids: Vec<u64> = dump.iter().map(|(id, _)| id).collect();
    ids.sort();
    assert_eq!(ids, vec![0, 2, 3]);

    // Removing an absent / already-removed id is false, not an error.
    assert!(!dump.remove(1));
    assert!(!dump.remove(99));
}


#[test]
fn remove_many_counts_only_live_removals() {
    let mut dump = FonDump::from_records(vec![record(0), record(1), record(2)]);
    let removed = dump.remove_many(&[0, 2, 2, 99]); // 2 listed twice, 99 absent
    assert_eq!(removed, 2);
    assert_eq!(dump.len(), 1);
    assert_eq!(id_of(dump.get(1).unwrap()), 1);
}


#[test]
fn replace_swaps_existing_record() {
    let mut dump = FonDump::from_records(vec![record(0), record(1)]);
    dump.replace(1, record(99)).expect("replace existing id");
    assert_eq!(dump.len(), 2);
    assert_eq!(id_of(dump.get(1).unwrap()), 99);
    assert_eq!(id_of(dump.get(0).unwrap()), 0);
}

#[test]
fn replace_absent_id_errors() {
    let mut dump = FonDump::from_records(vec![record(0)]);
    let err = dump.replace(5, record(5));
    assert!(err.is_err());
    // Replacing a tombstoned id also errors.
    dump.remove(0);
    assert!(dump.replace(0, record(0)).is_err());
}


#[test]
fn record_gen_bumps_on_remove_and_replace_only() {
    let mut dump = FonDump::from_records(vec![record(0), record(1)]);
    dump.ensure_mutable();
    let g0 = dump.record_gen(1).unwrap();

    // A tree-gen bump does NOT change record_gen (so a record view survives field edits).
    assert!(dump.bump_tree_gen(1));
    assert_eq!(dump.record_gen(1).unwrap(), g0);
    assert_eq!(dump.tree_gen(1).unwrap(), 1);

    // replace bumps record_gen.
    dump.replace(1, record(2)).unwrap();
    assert_eq!(dump.record_gen(1).unwrap(), g0 + 1);

    // remove makes both generations unreadable (absent).
    assert!(dump.remove(1));
    assert!(dump.record_gen(1).is_none());
    assert!(dump.tree_gen(1).is_none());
    assert!(!dump.bump_tree_gen(1)); // can't bump an absent record
}

#[test]
fn gens_readable_on_dense_without_mutation() {
    // A never-mutated Dense dump reports generation 0 for present ids, None otherwise.
    let dump = FonDump::from_records(vec![record(0), record(1)]);
    assert_eq!(dump.record_gen(0), Some(0));
    assert_eq!(dump.tree_gen(1), Some(0));
    assert_eq!(dump.record_gen(2), None);
}


#[test]
fn set_overwrites_existing_field_in_place() {
    let mut c = FonCollection::new();
    c.add("width", FonValue::Int(10));
    c.add("name", FonValue::String("a".to_string()));
    let len_before = c.len();

    c.set("width", FonValue::Int(20)).expect("set existing key");

    assert_eq!(c.len(), len_before); // no field added/removed
    assert!(matches!(c.get("width"), Some(FonValue::Int(20))));
    // Setting an absent key errors (use add_field to create).
    assert!(c.set("missing", FonValue::Int(0)).is_err());
}

#[test]
fn add_field_appends_new_key_and_rejects_duplicates() {
    let mut c = FonCollection::new();
    c.add_field("a", FonValue::Int(1)).expect("add new key");
    assert!(matches!(c.get("a"), Some(FonValue::Int(1))));
    assert!(c.add_field("a", FonValue::Int(2)).is_err()); // duplicate
}

#[test]
fn add_field_rejects_keys_that_cannot_round_trip() {
    let mut c = FonCollection::new();
    for key in ["a=b", "a\"b", "a{b", "a}b", "a[b", "a]b", "a\nb", "a\rb"] {
        assert!(c.add_field(key, FonValue::Int(1)).is_err(), "key {key:?} accepted");
    }
    assert!(c.is_empty());
    // Everything else is allowed, including U+0000, commas and colons.
    c.add_field("a\0b,c:d", FonValue::Int(1)).expect("plain key");
    let line = fon::serialize_to_string(&c);
    let back = fon::deserialize_line(line.as_bytes(), &fon::DeserializeOptions::default()).unwrap();
    assert!(matches!(back.get("a\0b,c:d"), Some(FonValue::Int(1))));
}

#[test]
fn remove_field_drops_key() {
    let mut c = FonCollection::new();
    c.add("a", FonValue::Int(1));
    c.add("b", FonValue::Int(2));

    assert!(c.remove_field("a"));
    assert!(c.get("a").is_none());
    assert!(matches!(c.get("b"), Some(FonValue::Int(2))));
    assert!(!c.remove_field("a")); // already gone
}
