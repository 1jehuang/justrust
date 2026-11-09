fn main() {
    let v: Vec<i64> = (1..=20).collect();
    let evens: Vec<i64> = v.iter().copied().filter(|x| x % 2 == 0).map(|x| x * x).collect();
    println!("{:?}", evens);
    let sum: i64 = v.iter().rev().skip(3).step_by(2).take(5).sum();
    println!("sum={sum}");
    let (small, big): (Vec<i64>, Vec<i64>) = v.iter().partition(|&&x| x < 10);
    println!("{} {}", small.len(), big.len());
    let zipped: Vec<(i64, char)> = v.iter().cloned().zip("abcde".chars()).collect();
    println!("{:?}", zipped);
    let pos = v.iter().position(|&x| x == 13);
    println!("{:?} {:?}", pos, v.iter().max_by_key(|&&x| (x % 7, x)));
    let words = ["alpha", "beta", "gamma", "delta"];
    let joined = words.iter().enumerate().map(|(i, w)| format!("{i}:{w}")).collect::<Vec<_>>().join(",");
    println!("{joined}");
    let flat: Vec<char> = words.iter().flat_map(|w| w.chars().take(2)).collect();
    println!("{}", flat.iter().collect::<String>());
    let folded = v.iter().fold(String::new(), |mut acc, x| { if x % 5 == 0 { acc.push_str(&x.to_string()); acc.push(' '); } acc });
    println!("[{}]", folded.trim_end());
    println!("{}", v.windows(3).filter(|w| w[0] + w[2] == 2 * w[1]).count());
    println!("{:?}", v.chunks(6).map(|c| c.iter().product::<i64>() % 1000).collect::<Vec<_>>());
    println!("{}", v.iter().any(|&x| x > 19) && v.iter().all(|&x| x > 0));
    let mut it = v.iter().peekable();
    while let Some(x) = it.next() { if let Some(&&n) = it.peek() { if n % 10 == 0 { print!("{x}->{n} "); } } }
    println!();
}
