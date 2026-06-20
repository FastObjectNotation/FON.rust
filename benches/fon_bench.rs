//! Criterion benchmarks for the `fon` crate.
//!
//! Criterion runs a warm-up phase before measuring and reports a statistical
//! steady state, so the first (cold, uncached) iterations never enter the
//! reported numbers. The `file_cached` group reads a file that warm-up has
//! already pulled into the OS page cache — it measures cached read speed. For
//! the cold-vs-warm file gap see `examples/file_cold_warm.rs`.

use std::hint::black_box;
use std::time::Duration;

use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion, Throughput};

use fon::types::{FonCollection, FonDump, FonValue};
use fon::{
    deserialize_dump_from_bytes, deserialize_from_file, deserialize_line, serialize_dump_to_string,
    serialize_to_file, serialize_to_string, DeserializeOptions, RawData,
};


fn make_record(i: u64) -> FonCollection {
    let mut c = FonCollection::new();
    c.add("id".into(), FonValue::ULong(i));
    c.add("name".into(), FonValue::String(format!("record number {i}")));
    c.add("price".into(), FonValue::Double(i as f64 * 1.5));
    c.add("active".into(), FonValue::Bool(i.is_multiple_of(2)));
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


fn bench_serialize(c: &mut Criterion) {
    let mut group = c.benchmark_group("serialize");

    let single = make_record(42);
    group.bench_function("collection_to_string", |b| {
        b.iter(|| serialize_to_string(black_box(&single)))
    });

    for &n in &[1_000u64, 10_000] {
        let dump = make_dump(n);
        group.throughput(Throughput::Elements(n));
        group.bench_with_input(BenchmarkId::new("dump_to_string", n), &dump, |b, d| {
            b.iter(|| serialize_dump_to_string(black_box(d), 0))
        });
    }

    group.finish();
}


fn bench_deserialize(c: &mut Criterion) {
    let mut group = c.benchmark_group("deserialize");
    let opts = DeserializeOptions::default();

    let line = serialize_to_string(&make_record(42));
    group.bench_function("line", |b| {
        b.iter(|| deserialize_line(black_box(line.as_bytes()), black_box(&opts)).unwrap())
    });

    for &n in &[1_000u64, 10_000] {
        let text = serialize_dump_to_string(&make_dump(n), 0);
        group.throughput(Throughput::Elements(n));
        group.bench_with_input(BenchmarkId::new("dump_from_bytes", n), &text, |b, t| {
            b.iter(|| deserialize_dump_from_bytes(black_box(t.as_bytes()), 0, black_box(&opts)).unwrap())
        });
    }

    group.finish();
}


fn bench_z85(c: &mut Criterion) {
    let mut group = c.benchmark_group("z85");
    let size = 64 * 1024usize;
    let data: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();

    group.throughput(Throughput::Bytes(size as u64));
    group.bench_function("pack_64k", |b| {
        b.iter_batched(
            || RawData::from_bytes(data.clone()),
            |mut r| {
                r.pack();
                r
            },
            BatchSize::SmallInput,
        )
    });

    let encoded = {
        let mut r = RawData::from_bytes(data.clone());
        r.pack();
        r.encoded().to_owned()
    };
    group.throughput(Throughput::Bytes(size as u64));
    group.bench_function("unpack_64k", |b| {
        b.iter_batched(
            || RawData::from_encoded(encoded.clone()),
            |mut r| {
                r.unpack().unwrap();
                r
            },
            BatchSize::SmallInput,
        )
    });

    group.finish();
}


fn bench_file(c: &mut Criterion) {
    let opts = DeserializeOptions::default();
    let n = 10_000u64;
    let dump = make_dump(n);
    let path = std::env::temp_dir().join("fon_criterion_bench.fon");
    serialize_to_file(&dump, &path, 0).expect("write bench fixture");

    let mut group = c.benchmark_group("file_cached");
    group.throughput(Throughput::Elements(n));
    // Warm-up reads pull the file into the OS page cache; this is cached read speed.
    group.bench_function("deserialize_from_file_10k", |b| {
        b.iter(|| deserialize_from_file(black_box(&path), 0, black_box(&opts)).unwrap())
    });
    group.finish();

    std::fs::remove_file(&path).ok();
}


fn config() -> Criterion {
    // Keep a real warm-up (so cold first runs are excluded) but bound total time.
    Criterion::default()
        .sample_size(50)
        .warm_up_time(Duration::from_millis(500))
        .measurement_time(Duration::from_secs(2))
}


criterion_group! {
    name = benches;
    config = config();
    targets = bench_serialize, bench_deserialize, bench_z85, bench_file
}
criterion_main!(benches);
