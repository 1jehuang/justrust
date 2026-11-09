use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};
#[derive(Debug)]
struct Node { val: i32, children: RefCell<Vec<Rc<Node>>>, parent: RefCell<Weak<Node>> }
fn node(v: i32) -> Rc<Node> { Rc::new(Node { val: v, children: RefCell::new(vec![]), parent: RefCell::new(Weak::new()) }) }
fn sum(n: &Rc<Node>) -> i32 { n.val + n.children.borrow().iter().map(sum).sum::<i32>() }
struct Memo { cache: RefCell<HashMap<u64, u64>>, hits: Cell<u32> }
impl Memo { fn fib(&self, n: u64) -> u64 { if n < 2 { return n; } if let Some(&v) = self.cache.borrow().get(&n) { self.hits.set(self.hits.get() + 1); return v; } let r = self.fib(n - 1) + self.fib(n - 2); self.cache.borrow_mut().insert(n, r); r } }
fn main() {
    let root = node(1);
    for v in 2..5 { let c = node(v); *c.parent.borrow_mut() = Rc::downgrade(&root); root.children.borrow_mut().push(c); }
    let leaf = node(10); root.children.borrow()[0].children.borrow_mut().push(leaf.clone());
    println!("sum={} strong={} weak={}", sum(&root), Rc::strong_count(&root), Rc::weak_count(&root));
    let first = root.children.borrow()[0].clone();
    println!("parent of {} is {:?}", first.val, first.parent.borrow().upgrade().map(|p| p.val));
    println!("leaf rc={}", Rc::strong_count(&leaf));
    let shared = Rc::new(RefCell::new(vec![1, 2]));
    let a = Rc::clone(&shared); let b = Rc::clone(&shared);
    a.borrow_mut().push(3); b.borrow_mut().push(4);
    println!("{:?} {}", shared.borrow(), Rc::ptr_eq(&a, &b));
    println!("{}", shared.try_borrow_mut().is_ok());
    let g = shared.borrow(); println!("{}", shared.try_borrow_mut().is_err()); drop(g);
    let m = Memo { cache: RefCell::new(HashMap::new()), hits: Cell::new(0) };
    println!("fib={} hits={}", m.fib(50), m.hits.get());
    let w; { let tmp = Rc::new(5); w = Rc::downgrade(&tmp); println!("{:?}", w.upgrade()); } println!("{:?}", w.upgrade());
}
