fn fib_above(limit: i32) -> i32 {
    let mut a = 1;
    let mut b = 1;
    loop {
        if b > limit {
            break b;
        }
        let next = a + b;
        a = b;
        b = next;
    }
}

fn main() {
    let mut n = 0;
    let mut total = 0;
    while n < 6 {
        n = n + 1;
        if n % 2 == 0 {
            continue;
        }
        total = total + n;
    }
    if fib_above(10) == 13 && total == 9 {
        println!("loop values and continue compiled correctly");
    }
}
