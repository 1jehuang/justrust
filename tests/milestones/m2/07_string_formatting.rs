use std::fmt;
struct Table { rows: Vec<(String, f64, i32)> }
impl fmt::Display for Table {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        writeln!(f, "{:<8}|{:>9}|{:^7}", "name", "price", "qty")?;
        writeln!(f, "{}", "-".repeat(26))?;
        for (n, p, q) in &self.rows { writeln!(f, "{:<8}|{:>9.2}|{:^7}", n, p, q)?; }
        write!(f, "{:=>26}", "")
    }
}
#[derive(Debug, Default)] struct Cfg { name: String, opts: Vec<Option<u8>>, nested: (i8, &'static str) }
fn main() {
    let t = Table { rows: vec![("apple".into(), 1.5, 3), ("kiwi".into(), 12.255, 10), ("fig".into(), 100.0, 7)] };
    println!("{t}");
    println!("{:08.3} {:+} {:e} {:x} {:#X} {:#b} {:o}", 3.14159, 42, 1234.5, 255, 255, 5u8, 64);
    println!("{:>width$}|{:.*}|{:#?}", "r", 2, 1.23456, (1, "a"), width = 5);
    let c = Cfg { name: "x".into(), opts: vec![Some(1), None], nested: (-3, "q") };
    println!("{:?}", c);
    println!("{:#?}", c);
    println!("{:?}", Cfg::default());
    println!("{} {} {}", c.name, c.opts.len(), c.nested.1);
    let s = "Hello, Wörld! ünïcode";
    println!("{} {} {}", s.len(), s.chars().count(), s.to_uppercase());
    println!("{:?}", s.char_indices().filter(|(_, c)| !c.is_ascii()).collect::<Vec<_>>());
    println!("{:?}", s.split(|c: char| c == ',' || c == '!').map(str::trim).collect::<Vec<_>>());
    println!("{} {:?} {:?}", s.replace("l", "L"), s.find("Wö"), s.rfind('o'));
    println!("{:?}", "a=1;b=2;;c=3".split(';').filter(|p| !p.is_empty()).filter_map(|p| p.split_once('=')).collect::<Vec<_>>());
    let mut out = String::new();
    use std::fmt::Write;
    for i in 0..3 { write!(out, "[{i:>2}]").unwrap(); }
    println!("{out} {}", out.starts_with("[ 0"));
    println!("{:?} {:?}", "  pad  ".trim_start(), "xxabcxx".trim_matches('x'));
    println!("{}", format!("{:?}", "quote\"tab\t"));
    let bytes = "AZ".as_bytes(); println!("{:?} {}", bytes, String::from_utf8_lossy(&[72, 105]));
    println!("{}", i64::MIN); println!("{} {}", u128::MAX, -0.0f64); println!("{} {} {}", f64::NAN, 1.0f64/0.0, 1e21);
}
