use std::collections::BTreeMap;

#[inline(never)]
fn histogram(words: &[&str]) -> BTreeMap<usize, Vec<String>> {
    let mut by_len = BTreeMap::new();
    for word in words {
        by_len.entry(word.len()).or_insert_with(Vec::new).push(word.to_uppercase());
    }
    by_len
}

fn main() {
    let words: Vec<String> = std::env::args().chain(["alpha".into(), "be".into(), "gamma".into()]).skip(1).collect();
    let words: Vec<&str> = words.iter().map(String::as_str).collect();
    println!("{:?}", histogram(&words));
}
