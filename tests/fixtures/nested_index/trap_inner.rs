fn i(tag: i32, n: usize) -> usize { println!("index {}", tag); n }
fn main() {
    let m = [[0i64; 3]; 2];
    let x = m[i(1, 0)][i(2, 9)];
    println!("{}", x);
}
