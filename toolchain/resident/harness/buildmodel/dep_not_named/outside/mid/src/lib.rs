/// Holds rustc's copy of `dep::kept::<u32>`, and calls `dep::scale` from its own object.
pub fn twice(n: u32) -> u32 {
    dep::scale(n) * 2 + dep::kept::<u32>(n)
}
