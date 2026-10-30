fn main() {
    let n = std::env::args().count() as u8;
    println!("{}", dep::bump(n + 254));
}
