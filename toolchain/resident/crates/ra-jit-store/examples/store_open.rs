//! What opening a large store costs one process: its time, and the memory it holds after it
//! and at most. Fills a store of its own with `make`, then `open` measures.
//!
//!     store_open make <dir> <values> <value bytes> <threads>
//!     store_open open <dir> <lookups> [many]
//!
//! `many` reads the values in one `get_many` on every core rather than one by one.
//!
//! Run `make` with `RA_JIT_CACHE_SEAL_AT` and `RA_JIT_CACHE_MAX=off` to choose how large
//! a pack is and that none is evicted. Memory is read from `/proc/self/status`.

use std::{path::Path, time::Instant};

use ra_jit_store::{KEY, Store};
use sha2::{Digest, Sha256};

fn key(n: u64) -> [u8; KEY] {
    Sha256::digest(n.to_le_bytes()).into()
}

/// A value that compresses about as compiled code does: a third of it noise.
fn value(n: u64, len: usize) -> Vec<u8> {
    let mut state = n.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1;
    (0..len)
        .map(|at| {
            if at % 3 != 0 {
                return (at / 7) as u8;
            }
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state as u8
        })
        .collect()
}

/// `VmRSS` and `VmHWM`, in megabytes.
fn memory() -> (u64, u64) {
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let field = |name: &str| {
        let line = status.lines().find(|it| it.starts_with(name)).unwrap_or_default();
        let kb: u64 = line.split_whitespace().nth(1).and_then(|it| it.parse().ok()).unwrap_or(0);
        kb >> 10
    };
    (field("VmRSS:"), field("VmHWM:"))
}

fn main() {
    ra_jit_store::compiles_against("store_open");
    ra_jit_store::built_from("store_open");
    let args: Vec<String> = std::env::args().skip(1).collect();
    let number = |at: usize| args[at].parse::<u64>().expect("a number");
    let dir = Path::new(&args[1]);
    match args[0].as_str() {
        "make" => {
            let (values, len, threads) = (number(2), number(3) as usize, number(4));
            let store = Store::open_at(dir).expect("a store");
            std::thread::scope(|scope| {
                for thread in 0..threads {
                    let store = &store;
                    scope.spawn(move || {
                        for n in (thread..values).step_by(threads as usize) {
                            store.insert(&key(n), &value(n, len));
                        }
                    });
                }
            });
            store.seal();
        }
        "open" => {
            let lookups = number(2);
            let started = Instant::now();
            let store = Store::open_at(dir).expect("a store");
            let opened = started.elapsed();
            let (rss, peak) = memory();
            println!("open: {:.3} s, rss {rss} MB, peak {peak} MB", opened.as_secs_f64());
            let started = Instant::now();
            let keys: Vec<[u8; KEY]> = (0..lookups).map(|n| key(n * 7919)).collect();
            let found = if args.get(3).is_some_and(|it| it == "many") {
                let threads = std::thread::available_parallelism().map_or(1, |it| it.get());
                store.get_many(&keys, threads).iter().filter(|it| it.is_some()).count()
            } else {
                keys.iter().filter(|key| store.get(key).is_some()).count()
            };
            let (rss, peak) = memory();
            println!(
                "{lookups} lookups ({found} found): {:.3} s, rss {rss} MB, peak {peak} MB",
                started.elapsed().as_secs_f64()
            );
        }
        other => panic!("`{other}` is neither `make` nor `open`"),
    }
}
