use std::fmt::Write;
fn main() {
    let w = 7usize;
    let p = 2usize;
    let x = 3.14159f64;
    println!("{:>width$}|{:.*}|{:#?}", "r", 2, 1.23456, (1, "a"), width = 5);
    println!("[{0:1$}] [{0:<1$}] [{0:^1$}]", 42, w);
    println!("[{:w$.p$}] [{x:>w$.p$}] [{:.prec$}]", x, x, prec = 4);
    println!("[{:0w$}] [{:+.*}] [{:*^w$}]", 5, 3, x, "ab");
    let s = format!("{:>1$}|{:<w$}|{:.*}", "a", 4, 1, "xyz");
    println!("{s}");
    let mut out = String::new();
    write!(out, "{:03$}-{:.*}", 9, 1, 2.55f64, 4).unwrap();
    writeln!(out, "|{:>w$}", true).unwrap();
    print!("{out}");
    eprintln!("[{:>w$}]", "e");
}
