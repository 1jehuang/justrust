use std::collections::BTreeMap;
enum J { N, A(Vec<J>) }
enum K { N(i64), O(BTreeMap<String, K>) }
struct Tree { name: String, kids: Vec<Tree> }
fn depth(j: &J) -> usize { match j { J::A(v) => 1 + v.iter().map(depth).max().unwrap_or(0), J::N => 0 } }
fn sum(k: &K) -> i64 { match k { K::N(n) => *n, K::O(m) => m.values().map(sum).sum() } }
fn count(t: &Tree) -> usize { 1 + t.kids.iter().map(count).sum::<usize>() }
fn main() {
    let j = J::A(vec![J::N, J::A(vec![J::A(vec![])])]);
    println!("depth {}", depth(&j));
    let mut inner = BTreeMap::new();
    inner.insert(String::from("x"), K::N(2));
    inner.insert(String::from("y"), K::N(3));
    let mut m = BTreeMap::new();
    m.insert(String::from("a"), K::N(1));
    m.insert(String::from("b"), K::O(inner));
    let k = K::O(m);
    println!("sum {}", sum(&k));
    if let K::O(m) = &k { for key in m.keys() { println!("key {key}"); } }
    let t = Tree { name: String::from("root"), kids: vec![Tree { name: String::from("a"), kids: vec![] }, Tree { name: String::from("b"), kids: vec![Tree { name: String::from("c"), kids: vec![] }] }] };
    println!("{} has {} nodes", t.name, count(&t));
}
