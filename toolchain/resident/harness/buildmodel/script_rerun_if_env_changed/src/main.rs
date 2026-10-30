fn main() {
    println!("level {} loud {}", env!("LEVEL"), cfg!(loud));
}
