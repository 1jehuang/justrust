fn main() {
    let text = "alpha\nbeta\r\n\ngamma\r\n";
    for (i, l) in text.lines().enumerate() {
        println!("{i}:[{l}] {}", l.len());
    }
    let n: Vec<&str> = "one\ntwo".lines().collect();
    println!("{:?} {}", n, "".lines().count());
    println!("{:?}", "a\r\nb\r".lines().collect::<Vec<&str>>());
    let a: Option<i32> = Some(4);
    let b: Option<i32> = None;
    println!("{} {}", a.map_or(0, |x| x * 2), b.map_or(-1, |x| x * 2));
    println!("{} {}", a.map_or_else(|| 7, |x| x + 1), b.map_or_else(|| 7, |x| x + 1));
    println!("{} {} {:?} {:?}", a.unwrap_or_default(), b.unwrap_or_default(), a.and(Some("y")), b.and(Some("y")));
    let s: Option<String> = None;
    println!("[{}]", s.unwrap_or_default());
    let ok: Result<i32, String> = Ok(3);
    let err: Result<i32, String> = Err("bad".to_string());
    println!("{} {}", ok.clone().map_or(0, |x| x * 3), err.clone().map_or(0, |x| x * 3));
    println!("{} {}", ok.clone().map_or_else(|e| e.len() as i32, |x| x), err.clone().map_or_else(|e| e.len() as i32, |x| x));
    println!("{} {}", ok.clone().unwrap_or_default(), err.clone().unwrap_or_default());
    println!("{:?} {:?}", ok.and(Ok::<u8, String>(9)), err.and(Ok::<u8, String>(9)));
}
