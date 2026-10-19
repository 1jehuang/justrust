struct Grid { cells: [[i64; 3]; 2], flags: [[bool; 2]; 2] }
fn main() {
    let mut m = [[0i64; 3]; 3];
    for i in 0..3usize {
        for j in 0..3usize {
            m[i][j] = (i * 3 + j) as i64;
        }
    }
    let mut t = [[0i64; 3]; 3];
    for i in 0..3usize {
        for j in 0..3usize {
            t[j][i] = m[i][j];
        }
    }
    for i in 0..3usize {
        t[i][i] *= 10;
        t[i][0] += 1;
        t[0][i] -= 2;
        m[i][2] <<= 1;
        m[2][i] |= 64;
    }
    let mut g = Grid { cells: [[1i64; 3]; 2], flags: [[false; 2]; 2] };
    let k = 1usize;
    g.cells[k][2] = 9;
    g.cells[0][k] ^= 3;
    g.flags[k][k] = true;
    g.flags[0][k] |= true;
    let mut cube = [[[0u8; 2]; 2]; 2];
    let (a, b, c) = (1usize, 0usize, 1usize);
    cube[a][b][c] = 7;
    cube[a][b][c] += cube[a][b][c];
    let mut s = 0i64;
    for i in 0..3usize {
        for j in 0..3usize {
            s += m[i][j] * 100 + t[i][j];
        }
    }
    println!("{}", s);
    println!("{} {} {}", g.cells[1][2], g.cells[0][1], g.flags[1][1] && g.flags[0][1]);
    println!("{}", cube[1][0][1]);
}
