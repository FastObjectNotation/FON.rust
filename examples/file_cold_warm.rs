//! Cold-vs-warm file-read benchmark.
//!
//! Windows keeps recently-touched files in its standby cache, so a file that
//! was just written is already warm. This reports two numbers:
//!   * cold — the first read after writing
//!   * warm — steady-state cached reads
//!
//! User-space cannot drop the Windows file cache, so the "cold" figure here is
//! the first read after the write (already partly cached). For a TRUE cold
//! read, clear the standby list with RAMMap / EmptyStandbyList (admin) or
//! reboot, then run this once before anything else touches the file.
//!
//! Run: `cargo run --release --example file_cold_warm`

use std::time::{Duration, Instant};

use fon::types::{FonCollection, FonDump, FonValue};
use fon::{deserialize_from_file, serialize_to_file, DeserializeOptions};


fn make_record(i: u64) -> FonCollection {
    let mut c = FonCollection::new();
    c.add("id".into(), FonValue::ULong(i));
    c.add("name".into(), FonValue::String(format!("record number {i}")));
    c.add("price".into(), FonValue::Double(i as f64 * 1.5));
    c.add("active".into(), FonValue::Bool(i % 2 == 0));
    c.add("tags".into(), FonValue::IntArray(vec![1, 2, 3, 4, 5]));
    c
}


fn make_dump(n: u64) -> FonDump {
    let mut d = FonDump::new();
    for i in 0..n {
        d.add(i, make_record(i));
    }
    d
}


fn main() {
    let n = 100_000u64;
    let dump = make_dump(n);
    let path = std::env::temp_dir().join("fon_cold_warm.fon");
    serialize_to_file(&dump, &path, 0).expect("write fixture");

    let size_kb = std::fs::metadata(&path).map(|m| m.len() / 1024).unwrap_or(0);
    let opts = DeserializeOptions::default();

    let t0 = Instant::now();
    let first = deserialize_from_file(&path, 0, &opts).expect("cold read");
    let cold = t0.elapsed();

    let runs = 10u32;
    let mut warm_total = Duration::ZERO;
    for _ in 0..runs {
        let t = Instant::now();
        let d = deserialize_from_file(&path, 0, &opts).expect("warm read");
        warm_total += t.elapsed();
        std::hint::black_box(d);
    }
    let warm = warm_total / runs;

    println!("file: {} records, {size_kb} KB", first.len());
    println!("cold (1st read): {cold:?}");
    println!("warm (avg of {runs}): {warm:?}");
    if warm.as_secs_f64() > 0.0 {
        println!("cold/warm ratio: {:.2}x", cold.as_secs_f64() / warm.as_secs_f64());
    }
    println!();
    println!("note: user-space cannot drop the Windows file cache, so 'cold' is the");
    println!("first read after write. For a true cold read clear the standby list");
    println!("(RAMMap / EmptyStandbyList, admin) or reboot, then run this once.");

    std::fs::remove_file(&path).ok();
}
