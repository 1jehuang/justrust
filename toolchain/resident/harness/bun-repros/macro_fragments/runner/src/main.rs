fn main() {
    for tag in 0..7 {
        println!("{}", macro_fragments::TaskTag(tag).name());
    }
}
