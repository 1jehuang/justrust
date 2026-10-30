fn work(n: u32) -> u32 {
    #[cfg(feature = "tracing")]
    tracing::trace!("working");
    n + 1
}

fn main() {
    println!("{}", work(1));
}
