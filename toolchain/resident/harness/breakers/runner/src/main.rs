fn main() {
    let name = std::env::args().nth(1).unwrap_or_default();
    match breakers::run(&name) {
        Some(output) => println!("{output}"),
        None => println!("{}", breakers::NAMES.join("\n")),
    }
}
