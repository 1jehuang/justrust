use std::collections::HashMap;
fn main() {
    let text = "a b a c b a";
    let mut counts: HashMap<String, u32> = HashMap::new();
    for w in text.split_whitespace() { *counts.entry(w.to_string()).or_insert(0) += 1; }
    let mut keys: Vec<_> = counts.keys().cloned().collect();
    keys.sort();
    for k in &keys { println!("{} {}", k, counts[k]); }
    if let Some(v) = counts.get("a") { println!("a={}", v); }
    println!("{}", counts.contains_key("z"));
}
