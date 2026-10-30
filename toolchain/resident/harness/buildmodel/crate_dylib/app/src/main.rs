fn main() {
    let n = std::env::args().count() as u32;
    println!("{} {}", dep::triple(n), dep::greet("you"));
}
