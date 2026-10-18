fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 { a } else { gcd(b, a % b) }
}

fn lcm(a: u64, b: u64) -> u64 {
    a / gcd(a, b) * b
}

fn fib(n: u32) -> u64 {
    let (mut a, mut b) = (0u64, 1u64);
    for _ in 0..n {
        let t = a + b;
        a = b;
        b = t;
    }
    a
}

fn main() {
    println!("gcd(48, 18) = {}", gcd(48, 18));
    println!("lcm(4, 6) = {}", lcm(4, 6));
    let mut l = 1;
    for k in 1..=20 {
        l = lcm(l, k);
    }
    println!("lcm(1..=20) = {}", l);
    for n in [0, 1, 2, 10, 50, 90] {
        println!("fib({}) = {}", n, fib(n));
    }
}
