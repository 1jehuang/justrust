//! tests/ui/mir/mir_refs_correct.rs: a tuple variant's constructor cast to a pointer was
//! taken for a cast of the variant to its discriminant.

enum Wrapped {
    One(u8),
}

#[inline(never)]
fn constructor() -> fn(u8) -> Wrapped {
    Wrapped::One
}

pub fn run() -> (bool, bool, u8) {
    let same = constructor() as *mut () == Wrapped::One as *mut ();
    let null = Wrapped::One as usize == 0;
    let Wrapped::One(inside) = constructor()(7);
    (same, null, inside)
}
