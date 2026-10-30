//! A function that hands a large value to one call after another, and recurses. A callee
//! owns the argument it is handed, so the caller gives it a copy; ra-jit kept a copy per
//! call for as long as the caller ran, and a frame was twice what rustc's is. just's
//! `recursion_limit::bugfix` parses 255 nested parentheses on the main thread's 8 MiB: a
//! level of its parser was 19 KiB as rustc builds it and 33 KiB as ra-jit did, which
//! overflowed. Here a level is 16 values of 2 KiB, and 150 levels fit the stack once.

pub struct Wide([u64; 256]);

#[inline(never)]
fn made(seed: u64) -> Wide {
    let mut wide = Wide([0; 256]);
    for (at, slot) in wide.0.iter_mut().enumerate() {
        *slot = seed.wrapping_mul(at as u64 | 1);
    }
    wide
}

#[inline(never)]
fn taken(wide: Wide) -> u64 {
    wide.0.iter().fold(0, |sum, it| sum.rotate_left(3) ^ it)
}

fn level(depth: u32, seed: u64) -> u64 {
    if depth == 0 {
        return seed;
    }
    let mut sum = seed;
    sum ^= taken(made(sum));
    sum ^= taken(made(sum));
    sum ^= taken(made(sum));
    sum ^= taken(made(sum));
    sum ^= taken(made(sum));
    sum ^= taken(made(sum));
    sum ^= taken(made(sum));
    sum ^= taken(made(sum));
    sum ^= taken(made(sum));
    sum ^= taken(made(sum));
    sum ^= taken(made(sum));
    sum ^= taken(made(sum));
    sum ^= taken(made(sum));
    sum ^= taken(made(sum));
    sum ^= taken(made(sum));
    sum ^= taken(made(sum));
    sum ^ level(depth - 1, sum.wrapping_add(1))
}

/// On a thread with the 8 MiB the main thread has on Linux: on Windows it has 1 MiB, which
/// rustc's frames overflow as well.
pub fn run() -> u64 {
    let levels = std::thread::Builder::new().stack_size(8 << 20).spawn(|| level(150, 7));
    levels.expect("a thread").join().expect("the levels fit the stack")
}
