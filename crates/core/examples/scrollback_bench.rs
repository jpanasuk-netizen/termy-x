//! Feed throughput while output scrolls through history under row and byte
//! limits: a 117x32 engine fed 64 KiB chunks, best of several runs.
//!
//! cargo run --release -p termy_core --example scrollback_bench -- [MiB]
//! cargo run --release -p termy_core --example scrollback_bench -- \
//!     [MiB] <plain|long|unicode> <rows|unlimited> <byte limit MiB>

use std::{hint::black_box, time::Instant};
use termy_core::terminal_engine::{Engine, Options, Size};

const MIB: usize = 1024 * 1024;
const CHUNK: usize = 64 * 1024;
/// Best of this many fresh engines, as shared machines vary run to run.
const RUNS: usize = 7;

/// Deterministic log-like text: lines of 8..=180 printable bytes, so some wrap.
fn plain(target: usize) -> Vec<u8> {
    let words: &[&[u8]] = &[
        b"the",
        b"quick",
        b"brown",
        b"fox",
        b"jumps",
        b"over",
        b"lazy",
        b"dog",
        b"build",
        b"error:",
        b"warning",
        b"src/main.rs:42:7",
        b"0x7ffd1234",
        b"ok",
    ];
    let mut out = Vec::with_capacity(target + 256);
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    while out.len() < target {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        let len = 8 + (seed % 173) as usize;
        let start = out.len();
        let mut word = seed as usize;
        while out.len() - start < len {
            out.extend_from_slice(words[word % words.len()]);
            out.push(b' ');
            word = word.wrapping_mul(31).wrapping_add(7);
        }
        out.extend_from_slice(b"\r\n");
    }
    out
}

/// 1,500-byte lines, each wrapping over thirteen rows.
fn long_lines(target: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(target + 2048);
    while out.len() < target {
        for index in 0..1500 {
            out.push(b'a' + (index % 26) as u8);
        }
        out.extend_from_slice(b"\r\n");
    }
    out
}

/// Accented, CJK, Hangul and emoji text.
fn unicode(target: usize) -> Vec<u8> {
    let line = "ASCII café 日本語のテキスト 한글 🙂 emoji 界隈 Ελληνικά\r\n".as_bytes();
    let mut out = Vec::with_capacity(target + line.len());
    while out.len() < target {
        out.extend_from_slice(line);
    }
    out
}

fn run(label: &str, payload: &[u8], rows: usize, bytes: usize) {
    let mut best = 0f64;
    let mut history = 0;
    for _ in 0..RUNS {
        let (rate, size) = feed(payload, rows, bytes);
        best = best.max(rate);
        history = size;
    }
    let rows = if rows == usize::MAX {
        "unlimited".to_string()
    } else {
        rows.to_string()
    };
    println!(
        "{label:>7}  rows={rows:>9}  bytes={:>3} MiB  {best:>7.1} MB/s  history={history}",
        bytes / MIB,
    );
}

fn feed(payload: &[u8], rows: usize, bytes: usize) -> (f64, usize) {
    let mut engine = Engine::new(
        Size {
            cols: 117,
            rows: 32,
        },
        Options {
            scrollback_history: rows,
            scrollback_bytes: bytes,
            ..Options::default()
        },
    );
    let start = Instant::now();
    for chunk in payload.chunks(CHUNK) {
        engine.feed(black_box(chunk));
    }
    let elapsed = start.elapsed().as_secs_f64();
    black_box(engine.viewport_row(0));
    (payload.len() as f64 / 1e6 / elapsed, engine.history_size())
}

fn main() {
    let mib = std::env::args().nth(1).map_or(32, |value| {
        value.parse::<usize>().expect("MiB must be an integer")
    });
    let target = mib * MIB;
    let only: Vec<String> = std::env::args().skip(2).collect();
    if let [case, rows, bytes] = only.as_slice() {
        let payload = match case.as_str() {
            "plain" => plain(target),
            "long" => long_lines(target),
            "unicode" => unicode(target),
            _ => panic!("cases are plain, long and unicode"),
        };
        let rows = if rows == "unlimited" {
            usize::MAX
        } else {
            rows.parse().unwrap()
        };
        run(case, &payload, rows, bytes.parse::<usize>().unwrap() * MIB);
        return;
    }
    let configs = [
        (0, 16 * MIB),
        (1_000, 16 * MIB),
        (10_000, 16 * MIB),
        (10_000, 64 * MIB),
        (100_000, 64 * MIB),
        (usize::MAX, 16 * MIB),
        (usize::MAX, 64 * MIB),
    ];
    for (label, payload) in [
        ("plain", plain(target)),
        ("long", long_lines(target)),
        ("unicode", unicode(target)),
    ] {
        for (rows, bytes) in configs {
            run(label, &payload, rows, bytes);
        }
    }
}
