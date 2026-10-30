//! alloc-no-stdlib's macro for a pool on the stack expands to an array followed by a
//! semicolon, where an expression is expected.

#![allow(semicolon_in_expressions_from_macros)]

macro_rules! pool {
    ([$value:expr; $size:expr]) => {
        [$value; $size];
    };
}

fn fill(pool: &mut [u8]) -> usize {
    pool.iter_mut().enumerate().for_each(|(at, byte)| *byte = at as u8);
    pool.len()
}

pub fn run() -> (usize, u8, usize) {
    let mut pool = pool!([0; 64 - 8]);
    let filled = fill(&mut pool);
    (filled, pool[55], size_of_val(&pool))
}
