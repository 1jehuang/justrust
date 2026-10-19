use std::fmt::Write;
#[derive(Debug)]
struct P { x: i32, y: i32 }
fn main() {
    let x = 5;
    let p = P { x: 1, y: 2 };
    let name = "bob";
    let s = format!("x={} p={:?} n={} sum={}", x, p, name, x + 1);
    println!("{}", s);
    let t = format!("lit only");
    println!("{}", t);
    let w = format!("[{:>6}|{:<4}|{:^5}]", x, "ab", 1.5);
    println!("{}", w);
    let mut out = String::new();
    write!(out, "a{}", 1).unwrap();
    writeln!(out, " b{x}").unwrap();
    writeln!(out).unwrap();
    print!("{}", out);
    let v = vec![1, 2, 3];
    println!("{}", format!("{:?} {}", v, v.len()));
    eprintln!("err {}", s.len());
}
