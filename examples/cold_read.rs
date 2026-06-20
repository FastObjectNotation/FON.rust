//! Repeated UNCACHED (cold) read of a large FON file — for watching sustained
//! SSD read load in Task Manager.
//!
//! `FILE_FLAG_NO_BUFFERING` bypasses the OS page cache, so every pass hits the
//! disk (otherwise a machine with enough RAM would serve the whole file from
//! cache after the first read). The file is generated once and KEPT — delete it
//! yourself afterwards (the command is printed at the end).
//!
//! Reads are single-threaded sequential — the same shape as
//! `fon::deserialize_from_file` — so this shows the library's cold-read disk
//! load, not the array's peak (peak needs parallel / queued I/O).
//!
//! Usage: `cargo run --release --example cold_read -- [dir] [size_mb] [iters]`
//! defaults: system temp dir, 10240 MB (10 GB), 5 iterations.

use std::fs::File;
#[cfg(windows)]
use std::fs::OpenOptions;
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


#[cfg(windows)]
fn cold_read(path: &Path) -> std::io::Result<u64> {
    use std::os::windows::fs::OpenOptionsExt;

    const FILE_FLAG_NO_BUFFERING: u32 = 0x2000_0000;
    const FILE_FLAG_SEQUENTIAL_SCAN: u32 = 0x0800_0000;
    const SECTOR: usize = 4096;
    const CHUNK: usize = 32 * 1024 * 1024;

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
    let mut f = File::open(path)?;
    let mut buf = vec![0u8; 32 * 1024 * 1024];
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


fn main() {
    let mut args = std::env::args().skip(1);
    let dir = args.next().map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let size_mb: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(10240);
    let iters: u32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(5);
    std::fs::create_dir_all(&dir).expect("create dir");

    let target = size_mb * 1024 * 1024;
    let path = dir.join(format!("fon_cold_{size_mb}mb.fon"));

    let need = match std::fs::metadata(&path) {
        Ok(m) => m.len() < target,
        Err(_) => true,
    };
    if need {
        println!("generating {size_mb} MB at {} ...", path.display());
        let block = serialize_dump_to_string(&make_dump(5_000), 0).into_bytes();
        let w = generate(&path, target, &block).expect("generate");
        println!("wrote {:.0} MB", w as f64 / (1024.0 * 1024.0));
    } else {
        println!("reusing existing {}", path.display());
    }

    let mb = std::fs::metadata(&path).unwrap().len() as f64 / (1024.0 * 1024.0);
    println!("cold (uncached) reads x{iters} — watch the disk in Task Manager:");
    for i in 1..=iters {
        let t = Instant::now();
        let _ = cold_read(&path).expect("cold read");
        let s = t.elapsed().as_secs_f64();
        println!("  pass {i}: {mb:.0} MB in {s:.2} s  ({:.0} MB/s)", mb / s);
    }
    println!("done. delete with:  Remove-Item \"{}\"", path.display());
}
