//! tests/ui/coercion/trait-object-arrays-11205.rs: an array written `[value; N]` as the argument
//! of a call that takes an array of trait objects. ra-jit did not look through the argument's
//! expected type to its element, found a mismatch, and left the function to rustc.

trait Weight {
    fn weight(&self) -> u32;
}

impl Weight for u8 {
    fn weight(&self) -> u32 {
        u32::from(*self)
    }
}

#[inline(never)]
fn total(items: [&dyn Weight; 3]) -> u32 {
    items.iter().map(|item| item.weight()).sum()
}

pub fn run() -> u32 {
    total([&7u8; 3])
}
