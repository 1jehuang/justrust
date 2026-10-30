//! Bytes nobody vouches for, for the tests of what reads the store: a pack, an entry and
//! LLVM's code are read from files and from other machines.
//!
//! [`inputs`] gives the files of a corpus directory, the seeds a test builds, and changes of
//! both, the same ones in every run. [`largest_allocation`] tells how much memory a read
//! asked for at once, so that a length in the bytes is seen not to be believed. A test binary
//! that links this crate allocates through its counting allocator: it is for tests alone.

use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
    path::Path,
};

/// How many changed inputs a target is given, unless `RA_JIT_FUZZ_RUNS` says.
const RUNS: usize = 20_000;

/// Bytes that are a length, a tag or an index somewhere, at their edges.
const EDGES: [u8; 8] = [0, 1, 2, 0x7f, 0x80, 0xfe, 0xff, 0x40];

/// The inputs for the corpus in `dir`: its files, `seeds`, and changes of them.
pub fn inputs(dir: impl AsRef<Path>, seeds: Vec<Vec<u8>>) -> impl Iterator<Item = Vec<u8>> {
    let dir = dir.as_ref();
    let target = dir.file_name().map_or(0, |it| it.len());
    let mut known = corpus(dir);
    known.extend(seeds);
    let runs = std::env::var("RA_JIT_FUZZ_RUNS").ok().and_then(|it| it.parse().ok());
    let mut random = Random(0x9e37_79b9_7f4a_7c15 ^ target as u64);
    let changed: Vec<Vec<u8>> = (0..runs.unwrap_or(RUNS))
        .map(|run| {
            let mut bytes = known[run % known.len()].clone();
            for _ in 0..=random.below(4) {
                random.change(&mut bytes, &known);
            }
            bytes
        })
        .collect();
    known.into_iter().chain(changed)
}

/// The files of `dir`, each the hexadecimal of an input. Space in them is for the reader.
fn corpus(dir: &Path) -> Vec<Vec<u8>> {
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|it| it.unwrap().path())
        .collect();
    files.sort();
    files.iter().map(|file| unhex(&std::fs::read_to_string(file).unwrap())).collect()
}

fn unhex(text: &str) -> Vec<u8> {
    let digits: Vec<u8> = text.bytes().filter(|it| !it.is_ascii_whitespace()).collect();
    digits
        .chunks(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}

/// xorshift64: the same numbers in every run.
struct Random(u64);

impl Random {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, bound: usize) -> usize {
        (self.next() % bound.max(1) as u64) as usize
    }

    fn edge(&mut self) -> u8 {
        EDGES[self.below(EDGES.len())]
    }

    /// Changes `bytes` in one of the ways a damaged or a made-up input differs from a
    /// written one.
    fn change(&mut self, bytes: &mut Vec<u8>, known: &[Vec<u8>]) {
        let at = self.below(bytes.len());
        match self.below(8) {
            0 => bytes.truncate(at),
            1 if !bytes.is_empty() => bytes[at] ^= 1 << self.below(8),
            2 if !bytes.is_empty() => bytes[at] = self.edge(),
            3 => bytes.insert(at, self.edge()),
            4 if !bytes.is_empty() => _ = bytes.remove(at),
            // A length that says there is far more than there is.
            5 => _ = bytes.splice(at..at, [0xff; 9]),
            6 => {
                let other = &known[self.below(known.len())];
                let from = self.below(other.len());
                bytes.splice(at.., other[from..].iter().copied());
            }
            _ => {
                for _ in 0..self.below(16) {
                    let byte = self.next() as u8;
                    bytes.insert(self.below(bytes.len() + 1), byte);
                }
            }
        }
    }
}

/// Counts the memory the tests of this crate ask for, a thread at a time.
struct Counting;

thread_local! {
    static LARGEST: Cell<usize> = const { Cell::new(0) };
}

// SAFETY: every request is the system allocator's to answer, as it was given.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        Counting::saw(layout.size());
        // SAFETY: the caller's layout.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, at: *mut u8, layout: Layout) {
        // SAFETY: the caller's pointer and layout.
        unsafe { System.dealloc(at, layout) }
    }

    unsafe fn realloc(&self, at: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        Counting::saw(size);
        // SAFETY: the caller's pointer, layout and size.
        unsafe { System.realloc(at, layout, size) }
    }
}

impl Counting {
    fn saw(size: usize) {
        // A thread that is ending has nothing to count in.
        _ = LARGEST.try_with(|largest| largest.set(largest.get().max(size)));
    }
}

#[global_allocator]
static COUNTING: Counting = Counting;

/// The most bytes `read` asked for at once, and what it returned.
pub fn largest_allocation<T>(read: impl FnOnce() -> T) -> (usize, T) {
    LARGEST.set(0);
    let read = read();
    (LARGEST.get(), read)
}

/// The most a read of `len` bytes may ask for at once: serde makes room for at most a
/// megabyte of what a length promises before the bytes are there.
pub fn allowed(len: usize) -> usize {
    (1 << 20) + 64 * len
}
