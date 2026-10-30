#[macros::traced]
fn work(n: usize) -> usize {
    n * 3
}

fn main() {
    println!("{}", work(std::env::args().count()));
}
