fn main() {
    let total: usize = (0..1000).map(|n| app::shared("bench").len() + n).sum();
    println!("bench: {total} debug_assertions: {}", cfg!(debug_assertions));
}
