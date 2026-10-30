//! zmij's test picks a constant with `if let b"0" = LEVEL.as_bytes()`: a byte string pattern
//! against a slice. The evaluator compared the two as sized values and gave up on `[u8]`.

const LEVEL: &str = "0";
const COUNT: usize = if let b"0" = LEVEL.as_bytes() { 1_000 } else { 100_000 };
const OTHER: usize = match LEVEL.as_bytes() {
    b"" => 1,
    b"00" => 2,
    b"1" => 3,
    _ => 4,
};

fn at_run_time(bytes: &[u8]) -> u8 {
    match bytes {
        b"" => 0,
        b"0" => 1,
        b"zero" => 2,
        _ => 3,
    }
}

fn array_at_run_time(bytes: &[u8; 2]) -> bool {
    matches!(bytes, b"ok")
}

pub fn run() -> (usize, usize, [u8; 4], [bool; 2]) {
    let level = std::hint::black_box("zero");
    (
        COUNT,
        OTHER,
        [at_run_time(b""), at_run_time(b"0"), at_run_time(level.as_bytes()), at_run_time(b"00")],
        [array_at_run_time(b"ok"), array_at_run_time(b"no")],
    )
}
