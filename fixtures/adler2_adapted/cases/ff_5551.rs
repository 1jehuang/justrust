// Adapted from the recurrence documented in Adler2 2.0.1 src/algo.rs.
// This is NOT unchanged Adler2: serial modular reduction replaces its SIMD-like
// four-lane/chunked implementation. See README.md and upstream/LICENSE-0BSD.
// Copyright (C) Jonas Schievink <jonasschievink@gmail.com>
fn adler2_adapted_update(bytes: &[u8], checksum: u32) -> u32 {
    let mut a: u32 = checksum % 65536;
    let mut b: u32 = checksum / 65536;
    let mut i: usize = 0;
    while i < bytes.len() {
        a = (a + bytes[i] as u32) % 65521;
        b = (b + a) % 65521;
        i = i + 1;
    }
    b * 65536 + a
}

fn main() {
let p0: [u8; 5551] = [255; 5551];
let mut state: u32 = 1;
state = adler2_adapted_update(&p0, state);
if state == 1443076749 { println!("ff_5551:1443076749"); } else { println!("CHECKSUM_MISMATCH"); }
}
