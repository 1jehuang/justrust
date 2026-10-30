fn main() {
    let n = std::env::args().count() as u32;
    println!("member: {} from {}", dep::scale(n), dep::origin());
    println!("registry: {}", user::through_registry(n));
}
