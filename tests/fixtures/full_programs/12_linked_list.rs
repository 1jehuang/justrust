#[derive(Debug)]
enum List {
    Cons(i32, Box<List>),
    Nil,
}

use List::{Cons, Nil};

impl List {
    fn sum(&self) -> i32 {
        match self {
            Cons(v, rest) => v + rest.sum(),
            Nil => 0,
        }
    }
    fn len(&self) -> usize {
        match self {
            Cons(_, rest) => 1 + rest.len(),
            Nil => 0,
        }
    }
}

struct Node {
    value: i32,
    next: Option<Box<Node>>,
}

struct LinkedList {
    head: Option<Box<Node>>,
    size: usize,
}

impl LinkedList {
    fn new() -> Self {
        LinkedList { head: None, size: 0 }
    }
    fn push_front(&mut self, value: i32) {
        let old = self.head.take();
        self.head = Some(Box::new(Node { value, next: old }));
        self.size += 1;
    }
    fn pop_front(&mut self) -> Option<i32> {
        self.head.take().map(|node| {
            self.head = node.next;
            self.size -= 1;
            node.value
        })
    }
    fn reverse(&mut self) {
        let mut prev: Option<Box<Node>> = None;
        let mut cur = self.head.take();
        while let Some(mut node) = cur {
            cur = node.next.take();
            node.next = prev;
            prev = Some(node);
        }
        self.head = prev;
    }
    fn to_vec(&self) -> Vec<i32> {
        let mut out = Vec::new();
        let mut cur = &self.head;
        while let Some(node) = cur {
            out.push(node.value);
            cur = &node.next;
        }
        out
    }
}

fn main() {
    let list = Cons(1, Box::new(Cons(2, Box::new(Cons(3, Box::new(Nil))))));
    println!("{:?}", list);
    println!("sum = {}, len = {}", list.sum(), list.len());
    let mut ll = LinkedList::new();
    for i in 1..=5 {
        ll.push_front(i * 10);
    }
    println!("list: {:?} (size {})", ll.to_vec(), ll.size);
    ll.reverse();
    println!("reversed: {:?}", ll.to_vec());
    println!("popped: {:?}", ll.pop_front());
    println!("after pop: {:?} (size {})", ll.to_vec(), ll.size);
}
