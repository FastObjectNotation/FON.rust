//! Parse-ceiling experiment: how fast can we turn bytes into parsed records if
//! we drop the per-record `HashMap` and borrow `&str` fields straight from the
//! buffer (zero String allocation for keys and string values).
//!
//! Compares three on the same in-memory buffer (all chunk-parallel where noted):
//!   1. current  — fon::deserialize_dump_from_bytes (HashMap + String, lib).
//!   2. chunk HashMap — chunk-aligned parallel parse into FonCollection (HashMap).
//!   3. chunk borrowed — chunk-aligned parallel parse into Vec<(&str, Val)>.
//! Global allocator = mimalloc (cheap baseline, per the research).
//!
//! Usage: `cargo run --release --example parse_ceiling -- [size_mb]`  (default 1024)

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


// ---- variant 2: chunk-parallel into FonCollection (HashMap) ----

fn parse_hashmap(data: &[u8], opts: &DeserializeOptions, n: usize) -> Vec<Vec<FonCollection>> {
    chunk_bounds(data, n)
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


// ---- variant 3: chunk-parallel into Vec<(&str, Val)>, borrowed ----

enum Val<'a> {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(&'a str),
    IntArr(Vec<i64>),
    FloatArr(Vec<f64>),
}


type Record<'a> = Vec<(&'a str, Val<'a>)>;


fn parse_val(data: &[u8], tc: u8) -> (Val<'_>, usize) {
    if !data.is_empty() && data[0] == b'[' {
        let mut j = 1;
        while j < data.len() && data[j] != b']' {
            j += 1;
        }
        let inner = &data[1..j];
        let elems = |is_float: bool| -> (Val, usize) {
            if inner.is_empty() {
                return if is_float {
                    (Val::FloatArr(Vec::new()), j + 1)
                } else {
                    (Val::IntArr(Vec::new()), j + 1)
                };
            }
            if is_float {
                let v = inner
                    .split(|&b| b == b',')
                    .map(|s| std::str::from_utf8(s).unwrap_or("").parse().unwrap_or(0.0))
                    .collect();
                (Val::FloatArr(v), j + 1)
            } else {
                let v = inner
                    .split(|&b| b == b',')
                    .map(|s| std::str::from_utf8(s).unwrap_or("").parse().unwrap_or(0))
                    .collect();
                (Val::IntArr(v), j + 1)
            }
        };
        elems(tc == b'f' || tc == b'd')
    } else if tc == b's' || tc == b'r' {
        let mut j = 1;
        while j < data.len() {
            if data[j] == b'"' && data[j - 1] != b'\\' {
                break;
            }
            j += 1;
        }
        let s = std::str::from_utf8(&data[1..j]).unwrap_or("");
        (Val::Str(s), j + 1)
    } else {
        let mut j = 0;
        while j < data.len() && data[j] != b',' && data[j] != b']' {
            j += 1;
        }
        let s = std::str::from_utf8(&data[..j]).unwrap_or("");
        let v = match tc {
            b'f' | b'd' => Val::Float(s.parse().unwrap_or(0.0)),
            b'b' => Val::Bool(data.first() != Some(&b'0')),
            _ => Val::Int(s.parse().unwrap_or(0)),
        };
        (v, j)
    }
}


fn parse_line_borrowed(line: &[u8]) -> Record<'_> {
    let mut out = Vec::new();
    let mut i = 0;
    let n = line.len();
    while i < n {
        let key_start = i;
        while i < n && line[i] != b'=' {
            i += 1;
        }
        if i >= n {
            break;
        }
        let key = std::str::from_utf8(&line[key_start..i]).unwrap_or("");
        i += 1;
        if i + 1 >= n {
            break;
        }
        let tc = line[i];
        i += 2;
        let (val, consumed) = parse_val(&line[i..], tc);
        out.push((key, val));
        i += consumed;
        if i < n && line[i] == b',' {
            i += 1;
        }
    }
    out
}


fn parse_borrowed(data: &[u8], n: usize) -> Vec<Vec<Record<'_>>> {
    chunk_bounds(data, n)
        .par_iter()
        .map(|&(s, e)| {
            let mut out = Vec::new();
            for line in data[s..e].split(|&b| b == b'\n') {
                if !line.is_empty() {
                    out.push(parse_line_borrowed(line));
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
    let t1 = t.elapsed().as_secs_f64();
    let r1 = dump.len();
    drop(dump);

    let t = Instant::now();
    let hm = parse_hashmap(&buf, &opts, cores);
    let t2 = t.elapsed().as_secs_f64();
    let r2: usize = hm.iter().map(|v| v.len()).sum();
    drop(hm);

    let t = Instant::now();
    let bw = parse_borrowed(&buf, cores);
    let t3 = t.elapsed().as_secs_f64();
    let r3: usize = bw.iter().map(|v| v.len()).sum();
    drop(bw);

    println!("buffer {mb:.0} MB, cores={cores}, allocator=mimalloc");
    println!("1. current (lib, HashMap):      {t1:.3} s  ({:.0} MB/s, {r1} rec)", mb / t1);
    println!("2. chunk-parallel HashMap:      {t2:.3} s  ({:.0} MB/s, {r2} rec)", mb / t2);
    println!("3. chunk-parallel borrowed Vec: {t3:.3} s  ({:.0} MB/s, {r3} rec)", mb / t3);
    println!("borrowed vs current: {:.1}x   borrowed vs chunk-HashMap: {:.1}x", t1 / t3, t2 / t3);
}
