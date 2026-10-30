//! A frame held an array twice: `let buffer = [0u8; N]` builds the array in a temporary and
//! copies it to the binding, and each had a slot of its own. rayon-core's
//! `stack_overflow_crash` recurses 32 frames of a megabyte in a stack of 48 and overflowed it
//! with ra-jit's frames. The temporary and the binding are one slot now.

fn recurse(depth: u32) -> u8 {
    let mut buffer = [0u8; 1024 * 1024];
    std::hint::black_box(&mut buffer);
    if depth > 0 { recurse(depth - 1) } else { buffer[0] }
}

pub fn run() -> u8 {
    let thread = std::thread::Builder::new().stack_size(24 * 1024 * 1024);
    thread.spawn(|| recurse(16)).unwrap().join().unwrap()
}
