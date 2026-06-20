//! Decomposes file-read cost into raw I/O vs FON parsing, for one file size.
//!
//!   raw cold  — uncached byte read (FILE_FLAG_NO_BUFFERING), no parsing
//!   raw warm  — cached byte read, no parsing
//!   full warm — fon::deserialize_from_file (cached read + parse into FonDump)
//!   parse     — full warm minus raw warm (the cost the library adds on top of I/O)
//!
//! Usage: `cargo run --release --example read_overhead -- [dir] [size_mb]`
//! defaults: system temp dir, 1024 MB.

use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use fon::serialize_dump_to_string;
use fon::types::{FonCollection, FonDump, FonValue};
use fon::{deserialize_from_file, DeserializeOptions};


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
    warm_read(path)
}


fn mb_s(mb: f64, d: std::time::Duration) -> f64 {
    mb / d.as_secs_f64()
}


fn main() {
    let mut args = std::env::args().skip(1);
    let dir = args.next().map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let size_mb: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(1024);
    std::fs::create_dir_all(&dir).expect("create dir");

    let target = size_mb * 1024 * 1024;
    let block = serialize_dump_to_string(&make_dump(5_000), 0).into_bytes();
    let path = dir.join(format!("fon_overhead_{size_mb}mb.fon"));
    let written = generate(&path, target, &block).expect("generate");
    let mb = written as f64 / (1024.0 * 1024.0);

    let t = Instant::now();
    let _ = cold_read(&path).expect("cold read");
    let raw_cold = t.elapsed();

    let _ = warm_read(&path).expect("prime cache");
    let t = Instant::now();
    let _ = warm_read(&path).expect("warm read");
    let raw_warm = t.elapsed();

    let opts = DeserializeOptions::default();
    let t = Instant::now();
    let dump = deserialize_from_file(&path, 0, &opts).expect("deserialize");
    let full = t.elapsed();
    let records = dump.len();
    drop(dump);

    let parse = full.saturating_sub(raw_warm);

    println!("file: {:.0} MB, {} records", mb, records);
    println!("raw read, cold (uncached): {:>9.3?}  ({:>5.0} MB/s)", raw_cold, mb_s(mb, raw_cold));
    println!("raw read, warm (cached):   {:>9.3?}  ({:>5.0} MB/s)", raw_warm, mb_s(mb, raw_warm));
    println!("deserialize_from_file:     {:>9.3?}  ({:>5.0} MB/s)", full, mb_s(mb, full));
    println!(
        "-> parsing (full - raw warm): {:>9.3?}  ({:.0}% of full)",
        parse,
        100.0 * parse.as_secs_f64() / full.as_secs_f64()
    );

    std::fs::remove_file(&path).ok();
}
