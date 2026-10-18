fn collatz_len(mut n: u64) -> u32 {
    let mut steps = 0;
    while n != 1 {
        n = if n % 2 == 0 { n / 2 } else { 3 * n + 1 };
        steps += 1;
    }
    steps
}

fn main() {
    let mut best = 0;
    let mut best_n = 0;
    for n in 1..10000u64 {
        let len = collatz_len(n);
        if len > best {
            best = len;
            best_n = n;
        }
    }
    println!("longest chain under 10000 starts at {} with {} steps", best_n, best);
    println!("collatz(27) = {}", collatz_len(27));
}
