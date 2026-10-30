//! A `const fn` that matches a byte slice against byte string patterns, evaluated in a
//! constant: `http::HeaderName::from_static` finds the standard headers so.

const fn code(name: &[u8]) -> u8 {
    match name {
        b"accept" => 1,
        b"age" => 2,
        b"host" => 3,
        _ => 0,
    }
}

const fn wide(units: &[u16]) -> bool {
    matches!(units, [104, 105] | [])
}

const CODES: [u8; 5] = [code(b"accept"), code(b"age"), code(b"host"), code(b"hos"), code(b"")];
const WIDE: [bool; 3] = [wide(&[104, 105]), wide(&[104]), wide(&[])];

pub fn run() -> (Vec<u8>, Vec<bool>, u8) {
    let name = String::from("host");
    (CODES.to_vec(), WIDE.to_vec(), code(name.as_bytes()))
}
