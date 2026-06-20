//! File-size I/O sweep for FON files.
//!
//! For each size it generates a `.fon` file by streaming a serialized block
//! (constant RAM — a 10 GB file never lives in memory), then measures:
//!   * write — sequential write throughput to the target disk.
//!   * cold  — UNCACHED read. On Windows the file is opened with
//!     `FILE_FLAG_NO_BUFFERING`, which bypasses the OS page cache, so this is
//!     true SSD read speed even when the machine has enough RAM to cache the
//!     whole file (the usual reason "the first run isn't cold" on Windows).
//!   * warm  — buffered read served from the OS page cache (primed first).
//!
//! Usage: `cargo run --release --example file_sizes -- [dir]`
//! (default dir: the system temp dir; pass an SSD path like `E:\.temp`).

use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use fon::serialize_dump_to_string;
use fon::types::{FonCollection, FonDump, FonValue};


fn make_dump(n: u64) -> FonDump {
    let mut d = FonDump::new();
    for i in 0..n {
        let mut c = FonCollection::new();
        c.add("id".into(), FonValue::ULong(i));
        c.add("name".into(), FonValue::String(format!("record number {i}")));
        c.add("price".into(), FonValue::Double(i as f64 * 1.5));
        c.add("active".into(), FonValue::Bool(i.is_multiple_of(2)));
        c.add("tags".into(), FonValue::IntArray(vec![1, 2, 3, 4, 5]));
        d.add(i, c);
    }
    d
}


fn generate(path: &Path, target: u64, block: &[u8]) -> std::io::Result<u64> {
    let f = File::create(path)?;
    let mut w = BufWriter::with_capacity(8 * 1024 * 1024, f);
    let mut written = 0u64;
    while written < target {
        w.write_all(block)?;
        written += block.len() as u64;
    }
    w.flush()?;
    Ok(written)
}


fn warm_read(path: &Path) -> std::io::Result<u64> {
    let mut f = File::open(path)?;
    let mut buf = vec![0u8; 8 * 1024 * 1024];
    let mut total = 0u64;
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        total += n as u64;
    }
    Ok(total)
}


#[cfg(windows)]
fn cold_read(path: &Path) -> std::io::Result<u64> {
    use std::os::windows::fs::OpenOptionsExt;

    const FILE_FLAG_NO_BUFFERING: u32 = 0x2000_0000;
    const FILE_FLAG_SEQUENTIAL_SCAN: u32 = 0x0800_0000;
    const SECTOR: usize = 4096;
    const CHUNK: usize = 8 * 1024 * 1024;

    let mut f = OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_NO_BUFFERING | FILE_FLAG_SEQUENTIAL_SCAN)
        .open(path)?;

    // FILE_FLAG_NO_BUFFERING needs a sector-aligned buffer; over-allocate and
    // slice to the next 4 KB boundary.
    let mut raw = vec![0u8; CHUNK + SECTOR];
    let off = (SECTOR - (raw.as_ptr() as usize % SECTOR)) % SECTOR;
    let buf = &mut raw[off..off + CHUNK];

    let mut total = 0u64;
    loop {
        let n = f.read(buf)?;
        if n == 0 {
            break;
        }
        total += n as u64;
        if n < buf.len() {
            break;
        }
    }
    Ok(total)
}


#[cfg(not(windows))]
fn cold_read(path: &Path) -> std::io::Result<u64> {
    // No portable uncached read; fall back to a buffered read.
    warm_read(path)
}


fn mb_per_s(bytes: u64, secs: f64) -> f64 {
    (bytes as f64 / (1024.0 * 1024.0)) / secs
}


fn main() {
    let dir = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    std::fs::create_dir_all(&dir).expect("create dir");

    let sizes: &[(u64, &str)] = &[
        (1 << 20, "1MB"),
        (10 << 20, "10MB"),
        (100 << 20, "100MB"),
        (500 << 20, "500MB"),
        (1 << 30, "1GB"),
        (5u64 << 30, "5GB"),
        (10u64 << 30, "10GB"),
    ];

    let block = serialize_dump_to_string(&make_dump(5_000), 0).into_bytes();

    println!("dir: {}", dir.display());
    println!(
        "{:>6}  {:>9}  {:>9}  {:>9}  {:>11}  {:>11}",
        "size", "write s", "cold s", "warm s", "cold MB/s", "warm MB/s"
    );

    for &(target, label) in sizes {
        let path = dir.join(format!("fon_size_{label}.fon"));

        let t = Instant::now();
        let written = generate(&path, target, &block).expect("generate");
        let write_s = t.elapsed().as_secs_f64();

        let t = Instant::now();
        let _ = cold_read(&path).expect("cold read");
        let cold_s = t.elapsed().as_secs_f64();

        let _ = warm_read(&path).expect("prime cache");
        let t = Instant::now();
        let _ = warm_read(&path).expect("warm read");
        let warm_s = t.elapsed().as_secs_f64();

        println!(
            "{:>6}  {:>9.3}  {:>9.3}  {:>9.3}  {:>11.0}  {:>11.0}",
            label,
            write_s,
            cold_s,
            warm_s,
            mb_per_s(written, cold_s),
            mb_per_s(written, warm_s)
        );

        std::fs::remove_file(&path).ok();
    }
}
