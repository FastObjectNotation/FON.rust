//! Single-threaded vs N-threaded UNCACHED read of a file, to tell whether the
//! few-hundred-MB/s cold-read ceiling is the SSD itself or just our
//! queue-depth-1 single-threaded pattern. Each thread reads a distinct
//! CHUNK-aligned region with `FILE_FLAG_NO_BUFFERING`.
//!
//! Usage: `cargo run --release --example parallel_read -- [dir] [size_mb] [threads]`
//! defaults: temp dir, 2048 MB, 8 threads.

use std::fs::File;
#[cfg(windows)]
use std::fs::OpenOptions;
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use fon::serialize_dump_to_string;
use fon::types::{FonCollection, FonDump, FonValue};


const CHUNK: usize = 32 * 1024 * 1024;
#[cfg(windows)]
const SECTOR: usize = 4096;


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
fn read_region(path: &Path, start: u64, len: u64) -> std::io::Result<u64> {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_FLAG_NO_BUFFERING: u32 = 0x2000_0000;
    const FILE_FLAG_SEQUENTIAL_SCAN: u32 = 0x0800_0000;

    let mut f = OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_NO_BUFFERING | FILE_FLAG_SEQUENTIAL_SCAN)
        .open(path)?;
    f.seek(SeekFrom::Start(start))?;

    let mut raw = vec![0u8; CHUNK + SECTOR];
    let off = (SECTOR - (raw.as_ptr() as usize % SECTOR)) % SECTOR;
    let buf = &mut raw[off..off + CHUNK];

    let mut total = 0u64;
    while total < len {
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
fn read_region(path: &Path, start: u64, len: u64) -> std::io::Result<u64> {
    let mut f = File::open(path)?;
    f.seek(SeekFrom::Start(start))?;
    let mut buf = vec![0u8; CHUNK];
    let mut total = 0u64;
    while total < len {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        total += n as u64;
    }
    Ok(total)
}


fn parallel_read(path: &Path, size: u64, threads: usize) -> u64 {
    let region = (size / threads as u64) / CHUNK as u64 * CHUNK as u64;
    std::thread::scope(|s| {
        let handles: Vec<_> = (0..threads)
            .map(|t| {
                let start = t as u64 * region;
                let len = if t == threads - 1 { size - start } else { region };
                s.spawn(move || read_region(path, start, len).unwrap_or(0))
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).sum()
    })
}


fn main() {
    let mut args = std::env::args().skip(1);
    let dir = args.next().map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let size_mb: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(2048);
    let threads: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(8);
    std::fs::create_dir_all(&dir).expect("create dir");

    let target = size_mb * 1024 * 1024;
    let path = dir.join(format!("fon_par_{size_mb}mb.fon"));
    if std::fs::metadata(&path).map(|m| m.len() < target).unwrap_or(true) {
        let block = serialize_dump_to_string(&make_dump(5_000), 0).into_bytes();
        generate(&path, target, &block).expect("generate");
    }
    let size = std::fs::metadata(&path).unwrap().len();
    let mb = size as f64 / (1024.0 * 1024.0);

    let t = Instant::now();
    let _ = read_region(&path, 0, size).expect("single read");
    let s1 = t.elapsed().as_secs_f64();

    let t = Instant::now();
    let _ = parallel_read(&path, size, threads);
    let sn = t.elapsed().as_secs_f64();

    println!("file: {mb:.0} MB on {}", path.display());
    println!("1 thread (QD1):   {s1:.2} s  ({:.0} MB/s)", mb / s1);
    println!("{threads} threads:        {sn:.2} s  ({:.0} MB/s)", mb / sn);
    println!("speedup: {:.1}x", s1 / sn);
    std::fs::remove_file(&path).ok();
}
