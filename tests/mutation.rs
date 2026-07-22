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
