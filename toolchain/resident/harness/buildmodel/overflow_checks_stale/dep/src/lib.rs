pub fn bump(x: u8) -> u8 {
    let words: Vec<String> = std::env::args().collect();
    if words.len() > 5 {
        println!("{words:?}");
    }
    x + 1
}
