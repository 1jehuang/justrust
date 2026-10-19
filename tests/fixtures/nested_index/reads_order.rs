fn i(tag: i32, n: usize) -> usize { println!("index {}", tag); n }
fn main() {
    let m = [[1i64, 2i64, 3i64], [4i64, 5i64, 6i64]];
    let x = m[i(7, 1)][i(8, 2)];
    println!("{}", x);
    let s = m[i(9, 0)][i(10, 1)] + m[1][i(11, 2)];
    println!("{}", s);
    let t = [[[1i64; 2]; 2]; 2];
    let y = t[i(12, 1)][i(13, 0)][i(14, 1)];
    println!("{}", y);
    let mut sum = 0i64;
    for k in 0..2usize { sum += m[k][k]; }
    println!("{}", sum);
}
