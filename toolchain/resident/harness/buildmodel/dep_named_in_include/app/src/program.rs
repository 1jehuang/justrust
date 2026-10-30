fn main() {
    let n = std::env::args().count() as u32;
    // `impl From<Meters> for u64` is dep's: unless dep is loaded, `into` has no impl to pick.
    let total: u64 = dep::Meters(n * 3).into();
    println!("{total} m");
}
