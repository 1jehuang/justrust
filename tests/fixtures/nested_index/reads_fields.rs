struct Grid { cells: [[u8; 3]; 2], k: usize }
fn i(tag: i32, n: usize) -> usize { println!("index {}", tag); n }
fn main() {
    let g = Grid { cells: [[1u8, 2u8, 3u8], [4u8, 5u8, 6u8]], k: 1 };
    let a = g.cells[i(1, 1)][i(2, 0)];
    println!("{}", a);
    let b = g.cells[g.k][2];
    println!("{}", b);
    let m = [[true, false], [false, true]];
    let mut c = 0;
    for r in 0..2usize { for s in 0..2usize { if m[r][s] { c += 1; } } }
    println!("{}", c);
    let mut acc = [[0i64; 2]; 2];
    let row = acc[i(5, 1)];
    acc[0] = row;
    println!("{}", acc[0][1]);
}
