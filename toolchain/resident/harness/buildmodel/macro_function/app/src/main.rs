fn main() {
    let total: u64 = macros::sum!(1 2 3 4);
    println!("{}", total + std::env::args().count() as u64);
}
