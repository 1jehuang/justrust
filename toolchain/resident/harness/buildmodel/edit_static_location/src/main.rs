struct Site {
    file: &'static str,
    line: u32,
}

fn work(n: u32) -> String {
    static SITE: Site = Site { file: file!(), line: line!() };
    format!("{}:{} {}", SITE.file, SITE.line, n + 1)
}

fn main() {
    println!("{}", work(1));
}
