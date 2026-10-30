//! tests/ui/associated-consts/associated-const-match-patterns.rs and
//! tests/ui/rfcs/rfc-2302-self-struct-ctor: a pattern that is a path starting at a type,
//! `<T>::C` or `<T as Trait>::C`, and a unit struct matched as `Self`. ra-jit resolved neither
//! as a pattern and left the function to rustc.

trait Named {
    const NAME: u8;
}

#[derive(PartialEq, Eq)]
struct Unit;

impl Unit {
    const OWN: u8 = 3;

    #[inline(never)]
    fn is_self(&self) -> bool {
        match *self {
            Self => true,
        }
    }
}

impl Named for Unit {
    const NAME: u8 = 9;
}

#[inline(never)]
fn class(value: u8) -> u8 {
    match value {
        <Unit>::OWN => 1,
        <Unit as Named>::NAME => 2,
        0..=<Unit>::OWN => 3,
        _ => 4,
    }
}

pub fn run() -> (u8, u8, u8, u8, bool) {
    (class(3), class(9), class(2), class(200), Unit.is_self())
}
