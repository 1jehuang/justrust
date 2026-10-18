fn main() {
    let mut n: u32 = 1;
    while n <= 5 {
        let mut fact: u64 = 1;
        let mut k: u64 = 1;
        while k <= n as u64 { fact = fact * k; k = k + 1; }
        println!("{}! = {}", n, fact);
        n = n + 1;
    }
}
