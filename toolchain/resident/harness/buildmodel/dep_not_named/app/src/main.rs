fn main() {
    let n = std::env::args().count() as u32;
    println!("{} and {} from {}", dep::scale(n), mid::twice(n), dep::origin());
}
