fn parse(s: &str) -> Option<i32> { s.parse::<i32>().ok() }
fn main() {
    let xs = ["1", "x", "42", "-7"];
    for s in xs.iter() {
        let v = parse(s).map(|n| n * 2).unwrap_or(0);
        println!("{} -> {}", s, v);
    }
    let total: i32 = xs.iter().filter_map(|s| parse(s)).sum();
    println!("total {}", total);
    let r: Result<i32, String> = Err("bad".to_string());
    match r { Ok(v) => println!("{}", v), Err(e) => println!("err {}", e) }
}
