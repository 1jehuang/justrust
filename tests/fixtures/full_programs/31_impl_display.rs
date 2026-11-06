use std::fmt;
struct Matrix { rows: usize, cols: usize, data: Vec<f64> }
impl Matrix {
    fn new(rows: usize, cols: usize) -> Self { Matrix { rows, cols, data: vec![0.0; rows * cols] } }
    fn set(&mut self, r: usize, c: usize, v: f64) { self.data[r * self.cols + c] = v; }
}
impl fmt::Display for Matrix {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        for r in 0..self.rows {
            for c in 0..self.cols { write!(f, "{:6.2}", self.data[r * self.cols + c])?; }
            writeln!(f)?;
        }
        Ok(())
    }
}
fn main() {
    let mut m = Matrix::new(2, 3);
    m.set(0, 1, 3.5); m.set(1, 2, -1.25);
    print!("{}", m);
}
