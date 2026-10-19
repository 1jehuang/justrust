use std::fmt;
struct Pt { x: i32, y: i32 }
impl fmt::Display for Pt {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "({}, {})", self.x, self.y)
    }
}
struct L(Vec<i32>);
impl fmt::Display for L {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "len={}", self.0.len())?;
        write!(f, "end")
    }
}
fn main() {
    let p = Pt { x: 1, y: -2 };
    println!("{}", p);
    println!("[{}]", format!("{}", p));
    println!("{}", L(vec![1, 2]));
    let s = p.to_string();
    println!("{}", s);
}
