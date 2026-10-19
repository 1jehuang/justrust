fn i(tag: i32, n: usize) -> usize { println!("index {}", tag); n }
fn v(tag: i32, n: i64) -> i64 { println!("value {}", tag); n }
fn main() {
    let mut m = [[0i64; 3]; 2];
    m[i(1, 1)][i(2, 2)] = v(3, 7);
    m[i(4, 0)][i(5, 1)] += v(6, 5);
    println!("{} {}", m[1][2], m[0][1]);
    m[i(7, 9)][i(8, 0)] = v(9, 1);
}
