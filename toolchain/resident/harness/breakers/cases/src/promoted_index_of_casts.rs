//! bun's `Target::default_conditions_map` returns `&[b"browser" as &[u8], b"module"][..]` from
//! a closure: the array is a constant rustc promotes to a static, and the slice of it outlives
//! the call. Built on the closure's stack instead, the slice pointed at a frame that was gone,
//! and bun's transpiler read a length that was a stack address.

#[inline(never)]
fn conditions(target: u8) -> &'static [&'static [u8]] {
    let of = |target: u8| match target {
        0 => &[b"node" as &[u8]][..],
        1 => &[b"browser" as &[u8], b"module"][..],
        _ => &[b"macro" as &[u8], b"bun", b"node"][..],
    };
    of(target)
}

#[inline(never)]
fn clobber(depth: u32) -> u64 {
    let mut words = [depth as u64; 64];
    for (i, word) in words.iter_mut().enumerate() {
        *word = word.wrapping_mul(i as u64 + 0x9e37_79b9);
    }
    if depth == 0 { words.iter().sum() } else { clobber(depth - 1) ^ words[7] }
}

pub fn run() -> Vec<String> {
    let all: Vec<_> = (0..3).map(conditions).collect();
    std::hint::black_box(clobber(8));
    all.iter()
        .map(|names| names.iter().map(|name| String::from_utf8_lossy(name).into_owned()).collect::<Vec<_>>().join(","))
        .collect()
}
