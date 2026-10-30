fn main() {
    println!("{} {}", feat::voice(), cfg!(feature = "wide"));
}
