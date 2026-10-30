//! Found in blaze's self-test: hir-ide's `<Ty as HirDisplay>::hir_fmt` matches on every kind
//! of type, each arm with its own temporaries, and Cranelift gave each local of the body room
//! of its own: a frame of 26 KB where rustc's, whose LLVM shares the memory of locals whose
//! storage never overlaps, has a few hundred bytes. `recursive_vars` prints a type of a hundred
//! and more nested references, and overflowed the 2 MB stack of its test thread. Locals whose
//! storage never overlaps share a slot now.

use std::hint::black_box;

/// Recurses `depth` times through a frame that keeps one table of 4 KB of eight at a time.
#[inline(never)]
pub fn tables(n: u32, depth: u32) -> u64 {
    let here = match n % 8 {
        0 => {
            let table = [n as u8 ^ 0; 4096];
            black_box(&table)[n as usize % 4096] as u64
        }
        1 => {
            let table = [n as u8 ^ 31; 4096];
            black_box(&table)[n as usize % 4096] as u64
        }
        2 => {
            let table = [n as u8 ^ 62; 4096];
            black_box(&table)[n as usize % 4096] as u64
        }
        3 => {
            let table = [n as u8 ^ 93; 4096];
            black_box(&table)[n as usize % 4096] as u64
        }
        4 => {
            let table = [n as u8 ^ 124; 4096];
            black_box(&table)[n as usize % 4096] as u64
        }
        5 => {
            let table = [n as u8 ^ 155; 4096];
            black_box(&table)[n as usize % 4096] as u64
        }
        6 => {
            let table = [n as u8 ^ 186; 4096];
            black_box(&table)[n as usize % 4096] as u64
        }
        _ => {
            let table = [n as u8 ^ 217; 4096];
            black_box(&table)[n as usize % 4096] as u64
        }
    };
    if depth == 0 { here } else { here + tables(n + 1, depth - 1) }
}

#[inline(never)]
fn table(seed: u8) -> [u8; 2048] {
    [seed; 2048]
}

/// As [`tables`], with each table of 2 KB a temporary, two at a time: lowering gives a temporary no storage markers,
/// so where it holds a value is where it is assigned to where it is last read.
#[inline(never)]
pub fn temporaries(n: u32, depth: u32) -> u64 {
    let here = match n % 8 {
        0 => black_box(table(n as u8 ^ 0))[n as usize % 2048] as u64,
        1 => black_box(table(n as u8 ^ 31))[n as usize % 2048] as u64,
        2 => black_box(table(n as u8 ^ 62))[n as usize % 2048] as u64,
        3 => black_box(table(n as u8 ^ 93))[n as usize % 2048] as u64,
        4 => black_box(table(n as u8 ^ 124))[n as usize % 2048] as u64,
        5 => black_box(table(n as u8 ^ 155))[n as usize % 2048] as u64,
        6 => black_box(table(n as u8 ^ 186))[n as usize % 2048] as u64,
        _ => black_box(table(n as u8 ^ 217))[n as usize % 2048] as u64,
    };
    if depth == 0 { here } else { here + temporaries(n + 1, depth - 1) }
}
