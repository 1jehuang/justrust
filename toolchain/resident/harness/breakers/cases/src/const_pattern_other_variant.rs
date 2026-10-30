//! h2 matches a header name against constants of an enum whose other variant holds bytes
//! behind a `NonNull<[u8]>`. A constant is never in that variant, but the comparison was
//! written out for every variant and had no way to compare the pointer.

use std::ptr::NonNull;

#[derive(PartialEq, Eq)]
enum Standard {
    Accept,
    Age,
}

struct Custom {
    bytes: NonNull<[u8]>,
}

impl PartialEq for Custom {
    fn eq(&self, other: &Custom) -> bool {
        unsafe { self.bytes.as_ref() == other.bytes.as_ref() }
    }
}

impl Eq for Custom {}

#[derive(PartialEq, Eq)]
enum Repr {
    Standard(Standard),
    Custom(Custom),
}

#[derive(PartialEq, Eq)]
struct Name {
    inner: Repr,
}

const ACCEPT: Name = Name { inner: Repr::Standard(Standard::Accept) };
const AGE: Name = Name { inner: Repr::Standard(Standard::Age) };

fn index(name: &Name) -> u8 {
    match *name {
        ACCEPT => 19,
        AGE => 21,
        _ => 0,
    }
}

pub fn run() -> [u8; 3] {
    let custom = Name { inner: Repr::Custom(Custom { bytes: NonNull::from(&mut [1u8, 2][..]) }) };
    [index(&ACCEPT), index(&AGE), index(&custom)]
}
