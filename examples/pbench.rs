//! Internal parse-throughput harness for the `parse-speed` branch.
//!
//! Uses mimalloc (the allocator the shipped cdylib will use), runs the parse
//! best-of-N (dropping each result so memory stays bounded), and reports the
//! best and median — the best run is the least thermally throttled, so it is
//! the most representative of true speed on this machine. No teardown wait.
//!
//! Usage: `cargo run --release --example pbench -- [size_mb] [reps]`  (1024, 5)

use std::time::Instant;

use mimalloc::MiMalloc;

use fon::serialize_dump_to_string;
use fon::types::{FonCollection, FonDump, FonValue};
use fon::{deserialize_dump_from_bytes, DeserializeOptions};


#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;


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
    let mut args = std::env::args().skip(1);
    let size_mb: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(1024);
    let reps: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(5);
    let target = size_mb * 1024 * 1024;

    let block = serialize_dump_to_string(&make_dump(5_000), 0).into_bytes();
    let mut buf = Vec::with_capacity(target + block.len());
    while buf.len() < target {
        buf.extend_from_slice(&block);
    }
    let mb = buf.len() as f64 / (1024.0 * 1024.0);
    let opts = DeserializeOptions::default();

    let mut times = Vec::with_capacity(reps);
    let mut records = 0;
    for _ in 0..reps {
        let t = Instant::now();
        let dump = deserialize_dump_from_bytes(&buf, 0, &opts).expect("parse");
        let el = t.elapsed().as_secs_f64();
        records = dump.len();
        times.push(el);
        drop(dump);
    }
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let best = times[0];
    let median = times[times.len() / 2];

    println!("{mb:.0} MB, {records} records, reps={reps}, mimalloc");
    println!(
        "best:   {best:.3} s  ({:.0} MB/s, {:.2} M rec/s)",
        mb / best,
        records as f64 / 1e6 / best
    );
    println!(
        "median: {median:.3} s  ({:.0} MB/s, {:.2} M rec/s)",
        mb / median,
        records as f64 / 1e6 / median
    );
    std::process::exit(0);
}
