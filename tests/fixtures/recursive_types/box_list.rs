struct D(i32);
impl Drop for D { fn drop(&mut self) { println!("drop {}", self.0); } }
enum List { Cons(i32, Box<List>), Nil }
fn sum(l: &List) -> i32 { match l { List::Cons(v, n) => *v + sum(n.as_ref()), List::Nil => 0 } }
struct Node { v: i32, next: Option<Box<Node>> }
fn len(n: &Node) -> i32 { match &n.next { Some(b) => 1 + len(b.as_ref()), None => 1 } }
fn main() {
    let l = List::Cons(1, Box::new(List::Cons(2, Box::new(List::Nil))));
    let s = sum(&l);
    println!("{}", s);
    let n = Node { v: 1, next: Some(Box::new(Node { v: 2, next: Some(Box::new(Node { v: 3, next: None })) })) };
    let k = len(&n);
    println!("{} {}", k, n.v);
    let _e = Box::new(D(2));
}
