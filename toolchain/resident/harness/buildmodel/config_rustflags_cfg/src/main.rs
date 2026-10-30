fn main() {
    let n = std::env::args().count() as u8;
    let sum = std::panic::catch_unwind(move || 254 + n + n);
    println!("assertions {} sum {:?}", cfg!(debug_assertions), sum.ok());
}
