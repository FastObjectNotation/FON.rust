//! Prototype: chunk-aligned parallel parse vs the current `deserialize_dump_from_bytes`.
//!
//! Approach (the standard one for line-based formats):
//!   1. cut the buffer into N ~equal byte ranges (N = cores),
//!   2. snap each cut forward to the next '\n' so no record is split,
//!   3. parse each chunk's lines in parallel (Rayon work-stealing),
//!   4. keep per-chunk Vecs (no single-threaded HashMap assembly).
//! Also swaps the global allocator to mimalloc to cut allocator contention on
//! the millions of tiny per-record allocations.
//!
//! Usage: `cargo run --release --example parse_parallel -- [size_mb]`  (default 1024)

use std::time::Instant;

use mimalloc::MiMalloc;
use rayon::prelude::*;

use fon::serialize_dump_to_string;
use fon::types::{FonCollection, FonDump, FonValue};
use fon::{deserialize_dump_from_bytes, deserialize_line, DeserializeOptions};


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


fn chunk_bounds(data: &[u8], n: usize) -> Vec<(usize, usize)> {
    let len = data.len();
    if len == 0 || n <= 1 {
        return vec![(0, len)];
    }
    let mut points = Vec::with_capacity(n + 1);
    points.push(0usize);
    for i in 1..n {
        let approx = (len * i / n).min(len);
        let nl = data[approx..]
            .iter()
            .position(|&b| b == b'\n')
            .map(|p| approx + p + 1)
            .unwrap_or(len);
        if *points.last().unwrap() < nl {
            points.push(nl);
        }
    }
    if *points.last().unwrap() != len {
        points.push(len);
    }
    points.windows(2).map(|w| (w[0], w[1])).collect()
}


fn parallel_parse(data: &[u8], opts: &DeserializeOptions, n: usize) -> Vec<Vec<FonCollection>> {
    let bounds = chunk_bounds(data, n);
    bounds
        .par_iter()
        .map(|&(s, e)| {
            let mut out = Vec::new();
            for line in data[s..e].split(|&b| b == b'\n') {
                if !line.is_empty() {
                    out.push(deserialize_line(line, opts).unwrap());
                }
            }
            out
        })
        .collect()
}


fn main() {
    let size_mb: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(1024);
    let target = size_mb * 1024 * 1024;

    let block = serialize_dump_to_string(&make_dump(5_000), 0).into_bytes();
    let mut buf = Vec::with_capacity(target + block.len());
    while buf.len() < target {
        buf.extend_from_slice(&block);
    }
    let mb = buf.len() as f64 / (1024.0 * 1024.0);

    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(8);
    let opts = DeserializeOptions::default();

    let t = Instant::now();
    let dump = deserialize_dump_from_bytes(&buf, 0, &opts).expect("current");
    let t_cur = t.elapsed().as_secs_f64();
    let rec_cur = dump.len();
    drop(dump);

    let t = Instant::now();
    let parts = parallel_parse(&buf, &opts, cores);
    let t_par = t.elapsed().as_secs_f64();
    let rec_par: usize = parts.iter().map(|v| v.len()).sum();
    drop(parts);

    println!("buffer: {mb:.0} MB, cores={cores}, allocator=mimalloc");
    println!(
        "current deserialize_dump_from_bytes: {t_cur:.3} s  ({:.0} MB/s, {} records)",
        mb / t_cur,
        rec_cur
    );
    println!(
        "prototype chunk-parallel ({cores} chunks):  {t_par:.3} s  ({:.0} MB/s, {} records)",
        mb / t_par,
        rec_par
    );
    println!("speedup: {:.1}x", t_cur / t_par);
}
