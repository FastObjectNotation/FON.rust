//! Measures FON parse throughput (`deserialize_dump_from_bytes`, in-memory, no
//! I/O) so you can see how it scales with cores. Run it twice — once with
//! `RAYON_NUM_THREADS=1` and once unset (all cores) — and compare, while
//! watching the CPU in Task Manager.
//!
//! Usage: `cargo run --release --example parse_scaling -- [size_mb]`  (default 500)

use std::time::Instant;

use fon::serialize_dump_to_string;
use fon::types::{FonCollection, FonDump, FonValue};
use fon::{deserialize_dump_from_bytes, DeserializeOptions};


fn make_dump(n: u64) -> FonDump {
    let mut d = FonDump::new();
    for i in 0..n {
        let mut c = FonCollection::new();
        c.add("id".into(), FonValue::ULong(i));
        c.add("name".into(), FonValue::String(format!("record number {i}")));
        c.add("price".into(), FonValue::Double(i as f64 * 1.5));
        c.add("active".into(), FonValue::Bool(i % 2 == 0));
        c.add("tags".into(), FonValue::IntArray(vec![1, 2, 3, 4, 5]));
        d.add(i, c);
    }
    d
}


fn main() {
    let size_mb: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(500);
    let target = size_mb * 1024 * 1024;

    let block = serialize_dump_to_string(&make_dump(5_000), 0).into_bytes();
    let mut buf = Vec::with_capacity(target + block.len());
    while buf.len() < target {
        buf.extend_from_slice(&block);
    }
    let mb = buf.len() as f64 / (1024.0 * 1024.0);

    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(0);
    let rayon_env = std::env::var("RAYON_NUM_THREADS").unwrap_or_else(|_| "(all)".into());
    let opts = DeserializeOptions::default();

    let t = Instant::now();
    let dump = deserialize_dump_from_bytes(&buf, 0, &opts).expect("parse");
    let el = t.elapsed();

    println!("cores={cores}, RAYON_NUM_THREADS={rayon_env}");
    println!(
        "parse {:.0} MB ({} records): {:.3} s  ({:.0} MB/s, {:.2} M rec/s)",
        mb,
        dump.len(),
        el.as_secs_f64(),
        mb / el.as_secs_f64(),
        dump.len() as f64 / 1e6 / el.as_secs_f64()
    );
}
