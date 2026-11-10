use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

struct Node {
    val: i32,
    visits: Cell<u32>,
    marked: Cell<bool>,
    children: RefCell<Vec<Rc<Node>>>,
    parent: RefCell<Weak<Node>>,
}

impl Drop for Node {
    fn drop(&mut self) {
        println!("drop node {} visits {}", self.val, self.visits.get());
    }
}

fn node(v: i32) -> Rc<Node> {
    Rc::new(Node {
        val: v,
        visits: Cell::new(0),
        marked: Cell::new(false),
        children: RefCell::new(vec![]),
        parent: RefCell::new(Weak::new()),
    })
}

fn sum(n: &Rc<Node>) -> i32 {
    n.visits.set(n.visits.get() + 1);
    n.val + n.children.borrow().iter().map(|c| sum(c)).sum::<i32>()
}

fn main() {
    let root = node(1);
    for v in 2..5 {
        let c = node(v);
        *c.parent.borrow_mut() = Rc::downgrade(&root);
        root.children.borrow_mut().push(c);
    }
    let leaf = node(10);
    root.children.borrow()[0]
        .children
        .borrow_mut()
        .push(leaf.clone());
    println!(
        "sum={} strong={} weak={}",
        sum(&root),
        Rc::strong_count(&root),
        Rc::weak_count(&root)
    );
    println!("again={}", sum(&root));
    let first = root.children.borrow()[0].clone();
    first.marked.set(true);
    println!(
        "parent of {} is {:?} marked {} {}",
        first.val,
        first.parent.borrow().upgrade().map(|p| p.val),
        first.marked.get(),
        root.marked.get()
    );
    println!(
        "leaf rc={} visits={}",
        Rc::strong_count(&leaf),
        leaf.visits.get()
    );
    drop(root);
    println!(
        "orphan {} parent {:?}",
        first.val,
        first.parent.borrow().upgrade().map(|p| p.val)
    );
    println!("end");
}
