use std::collections::HashMap;
use std::collections::BTreeMap;

fn main() {
    let text = "the cat sat on the mat the cat ran";
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for w in text.split_whitespace() {
        *counts.entry(w).or_insert(0) += 1;
    }
    let mut sorted: Vec<(&str, usize)> = counts.into_iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    for (w, c) in &sorted {
        println!("{}: {}", w, c);
    }
    let mut by_len: BTreeMap<usize, Vec<&str>> = BTreeMap::new();
    for w in text.split_whitespace() {
        let entry = by_len.entry(w.len()).or_default();
        if !entry.contains(&w) {
            entry.push(w);
        }
    }
    for (len, words) in &by_len {
        println!("len {}: {:?}", len, words);
    }
    let mut scores = HashMap::new();
    scores.insert(String::from("blue"), 10);
    scores.insert(String::from("red"), 50);
    if let Some(s) = scores.get("blue") {
        println!("blue = {}", s);
    }
    scores.remove("blue");
    println!("has blue: {}, len: {}", scores.contains_key("blue"), scores.len());
}
