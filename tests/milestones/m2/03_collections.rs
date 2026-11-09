use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque, BinaryHeap};
use std::cmp::Reverse;
fn main() {
    let text = "the quick brown fox jumps over the lazy dog the end";
    let mut freq: HashMap<&str, usize> = HashMap::new();
    for w in text.split_whitespace() { *freq.entry(w).or_insert(0) += 1; }
    let mut pairs: Vec<_> = freq.into_iter().collect();
    pairs.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    println!("{:?}", &pairs[..3]);
    let mut bt = BTreeMap::new();
    for (i, c) in "mississippi".chars().enumerate() { bt.entry(c).or_insert_with(Vec::new).push(i); }
    for (k, v) in &bt { println!("{k}: {v:?}"); }
    println!("{:?}", bt.range('j'..'q').map(|(k, _)| *k).collect::<Vec<_>>());
    let a: HashSet<i32> = (1..10).collect(); let b: HashSet<i32> = (5..15).collect();
    let mut inter: Vec<_> = a.intersection(&b).copied().collect(); inter.sort();
    println!("{:?} {}", inter, a.union(&b).count());
    let s: BTreeSet<_> = [5, 1, 9, 1, 3].into_iter().collect();
    println!("{:?} first={:?} last={:?}", s, s.first(), s.last());
    let mut dq: VecDeque<i32> = VecDeque::new();
    for i in 0..5 { if i % 2 == 0 { dq.push_back(i) } else { dq.push_front(i) } }
    dq.rotate_left(2);
    println!("{:?} {:?}", dq, dq.iter().rev().collect::<Vec<_>>());
    let mut heap = BinaryHeap::new();
    for x in [7, 3, 9, 1, 4] { heap.push(Reverse(x)); }
    let mut out = vec![]; while let Some(Reverse(x)) = heap.pop() { out.push(x); }
    println!("{:?}", out);
    let mut v = vec![4, 1, 4, 2, 8, 2, 2];
    v.sort_unstable(); v.dedup(); v.retain(|&x| x != 8);
    println!("{:?} {:?}", v, v.binary_search(&4));
    let mut m: HashMap<String, Vec<u32>> = HashMap::new();
    m.insert("a".into(), vec![1]); m.get_mut("a").unwrap().push(2);
    if let Some(x) = m.remove("a") { println!("{x:?} {}", m.is_empty()); }
}
