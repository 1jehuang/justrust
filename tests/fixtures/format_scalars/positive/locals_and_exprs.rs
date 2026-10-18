fn square(x: i64) -> i64 { x * x }
fn main() {
    let a: i32 = 7;
    let b: u8 = 200;
    println!("a={} b={} sum={}", a, b, a + 3);
    println!("square={}", square(-12));
    let mut total: u64 = 0;
    let mut i: u64 = 1;
    while i <= 10 { total = total + i; i = i + 1; }
    println!("total {}", total);
    println!("{}{}{}", 1, 2, 3);
    println!("{}", -0i32);
}
