use std::collections::{BTreeMap, HashMap, HashSet};
const TEXT: &str = "It was the best of times, it was the worst of times,\nit was the age of wisdom, it was the age of foolishness,\nit was the epoch of belief, it was the epoch of incredulity,\nit was the season of Light, it was the season of Darkness.";
fn tokens(s: &str) -> impl Iterator<Item = String> + '_ { s.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(|w| w.to_lowercase()) }
fn main() {
    let mut index: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (ln, line) in TEXT.lines().enumerate() { for w in tokens(line) { let e = index.entry(w).or_default(); if e.last() != Some(&(ln + 1)) { e.push(ln + 1); } } }
    for (w, ls) in index.iter().filter(|(w, _)| w.len() > 5) { println!("{w:>12}: {ls:?}"); }
    let counts: HashMap<String, usize> = tokens(TEXT).fold(HashMap::new(), |mut m, w| { *m.entry(w).or_insert(0) += 1; m });
    let mut top: Vec<(&String, &usize)> = counts.iter().collect(); top.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
    println!("{:?}", top.iter().take(5).map(|(w, c)| format!("{w}={c}")).collect::<Vec<_>>());
    let bigrams: HashSet<(String, String)> = { let ws: Vec<String> = tokens(TEXT).collect(); ws.windows(2).map(|p| (p[0].clone(), p[1].clone())).collect() };
    let mut bs: Vec<_> = bigrams.iter().filter(|(a, _)| a == "the").map(|(_, b)| b.as_str()).collect(); bs.sort(); println!("the -> {:?}", bs);
    let longest = TEXT.lines().max_by_key(|l| l.len()).unwrap(); println!("{} chars: {}", longest.len(), &longest[..20]);
    let caps: String = TEXT.chars().filter(|c| c.is_uppercase()).collect(); println!("{caps}");
}
