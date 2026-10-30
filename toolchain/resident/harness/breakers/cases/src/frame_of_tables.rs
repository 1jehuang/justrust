//! curve25519-dalek's tests keep five tables of 950k together on the 2MB stack of a test thread,
//! each made by a trait function that fills a table of defaults and returns it, and whose
//! frame is two tables under rustc: the test has room for a slot a table and no more.

use std::hint::black_box;

#[derive(Clone, Copy)]
pub struct Row([u64; 125]);

impl Default for Row {
    fn default() -> Row {
        Row([0; 125])
    }
}

#[derive(Clone)]
pub struct Table([Row; 100]);

pub trait Create {
    type Seed;
    fn create(seed: &Self::Seed, above: usize, frame: &mut usize) -> Self;
}

pub type TableOfRows = Table;

impl Create for Table {
    type Seed = u64;

    #[inline(never)]
    fn create(seed: &u64, above: usize, frame: &mut usize) -> Table {
        let mut table = Table([Row::default(); 100]);
        let mut next = *seed;
        for i in 0..100 {
            table.0[i] = Row([next; 125]);
            next = next.wrapping_mul(3);
        }
        *frame = below(above);
        table
    }
}

/// How far under `above` the frame of this function is.
#[inline(never)]
fn below(above: usize) -> usize {
    let here = 0u8;
    above.wrapping_sub(black_box(&here as *const u8 as usize))
}

#[inline(never)]
fn sum(table: &Table) -> u64 {
    table.0.iter().map(|row| row.0[7]).fold(0, u64::wrapping_add)
}

#[inline(never)]
fn tables(above: usize) -> (u64, usize, usize) {
    let seed = &7;
    let mut made = 0;
    let first = TableOfRows::create(seed, 0, &mut made);
    let second = Table::create(seed, 0, &mut made);
    let third = Table::create(seed, above, &mut made);
    let all = sum(&first).wrapping_add(sum(&second)).wrapping_add(sum(&third));
    (all, made / 100_000, 0)
}

/// What the tables sum to, and whether the frames of `tables` and `create` take no more tables
/// of 100k together than rustc's three and two: fewer leave the test room too.
pub fn run() -> (u64, bool) {
    let top = 0u8;
    let (all, frames, _) = tables(black_box(&top as *const u8 as usize));
    (all, frames <= 5)
}
