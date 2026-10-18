fn sieve(limit: usize) -> Vec<usize> {
    let mut is_prime = vec![true; limit + 1];
    is_prime[0] = false;
    if limit >= 1 {
        is_prime[1] = false;
    }
    let mut i = 2;
    while i * i <= limit {
        if is_prime[i] {
            let mut j = i * i;
            while j <= limit {
                is_prime[j] = false;
                j += i;
            }
        }
        i += 1;
    }
    let mut primes = Vec::new();
    for (n, &p) in is_prime.iter().enumerate() {
        if p {
            primes.push(n);
        }
    }
    primes
}

fn main() {
    let primes = sieve(100);
    println!("{} primes below 100", primes.len());
    for p in &primes {
        print!("{} ", p);
    }
    println!();
    let big = sieve(100000);
    println!("count below 100000: {}", big.len());
    println!("largest: {}", big[big.len() - 1]);
}
