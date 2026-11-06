fn main() {
    let mut s = String::new();
    for i in 0..5 { s.push_str(&format!("{},", i)); }
    s.pop();
    println!("{}", s);
    let words: Vec<&str> = "the quick brown fox".split(' ').collect();
    let caps: Vec<String> = words.iter().map(|w| w.to_uppercase()).collect();
    println!("{}", caps.join("-"));
    println!("{}", s.len());
    let rev: String = "hello".chars().rev().collect();
    println!("{}", rev);
}
