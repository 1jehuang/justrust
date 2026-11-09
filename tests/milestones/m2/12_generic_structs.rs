use std::collections::HashMap;
use std::hash::Hash;
use std::fmt::Debug;
#[derive(Debug, Clone)] struct Matrix<const R: usize, const C: usize> { d: [[i32; C]; R] }
impl<const R: usize, const C: usize> Matrix<R, C> { fn transpose(&self) -> Matrix<C, R> { let mut d = [[0; R]; C]; for i in 0..R { for j in 0..C { d[j][i] = self.d[i][j]; } } Matrix { d } } }
struct Cache<K, V, F: Fn(&K) -> V> { f: F, map: HashMap<K, V> }
impl<K: Eq + Hash + Clone, V: Clone, F: Fn(&K) -> V> Cache<K, V, F> { fn new(f: F) -> Self { Cache { f, map: HashMap::new() } } fn get(&mut self, k: K) -> V { if let Some(v) = self.map.get(&k) { return v.clone(); } let v = (self.f)(&k); self.map.insert(k, v.clone()); v } }
#[derive(Debug, PartialEq)] enum Tree<T: Ord> { Leaf, Node(Box<Tree<T>>, T, Box<Tree<T>>) }
impl<T: Ord + Debug + Copy> Tree<T> {
    fn insert(self, x: T) -> Tree<T> { match self { Tree::Leaf => Tree::Node(Box::new(Tree::Leaf), x, Box::new(Tree::Leaf)), Tree::Node(l, v, r) => if x < v { Tree::Node(Box::new(l.insert(x)), v, r) } else { Tree::Node(l, v, Box::new(r.insert(x))) } } }
    fn inorder(&self, out: &mut Vec<T>) { if let Tree::Node(l, v, r) = self { l.inorder(out); out.push(*v); r.inorder(out); } }
    fn depth(&self) -> usize { match self { Tree::Leaf => 0, Tree::Node(l, _, r) => 1 + l.depth().max(r.depth()) } }
}
fn group_by<T, K: Ord + Clone, F: Fn(&T) -> K>(items: Vec<T>, key: F) -> std::collections::BTreeMap<K, Vec<T>> { let mut m = std::collections::BTreeMap::new(); for it in items { m.entry(key(&it)).or_insert_with(Vec::new).push(it); } m }
fn main() {
    let m = Matrix { d: [[1, 2, 3], [4, 5, 6]] };
    println!("{:?}", m.transpose());
    let mut calls = 0; let mut c = Cache::new(|x: &u64| (1..=*x).product::<u64>()); for k in [5, 10, 5, 10, 3] { calls += 1; print!("{} ", c.get(k)); } println!("{calls} {}", c.map.len());
    let t = [5, 2, 8, 1, 9, 3].iter().fold(Tree::Leaf, |t, &x| t.insert(x));
    let mut v = vec![]; t.inorder(&mut v); println!("{:?} depth={}", v, t.depth());
    println!("{:?}", group_by(vec!["apple", "avocado", "bean", "beet", "corn"], |s| s.chars().next().unwrap()));
    let pairs: Vec<(String, usize)> = vec![("a".into(), 1)]; let lookup: HashMap<_, _> = pairs.into_iter().collect(); println!("{:?}", lookup);
}
