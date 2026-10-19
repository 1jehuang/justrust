struct D(i32);
impl Drop for D { fn drop(&mut self) { println!("drop {}", self.0); } }
enum T { Leaf(D), Node(Box<T>, Box<T>) }
fn count(t: &T) -> i32 { match t { T::Leaf(_) => 1, T::Node(a, b) => count(a.as_ref()) + count(b.as_ref()) } }
struct A { x: Option<Box<A>>, d: D }
fn main() {
    let t = T::Node(Box::new(T::Leaf(D(1))), Box::new(T::Node(Box::new(T::Leaf(D(2))), Box::new(T::Leaf(D(3))))));
    let c = count(&t);
    println!("{}", c);
    let a = A { x: Some(Box::new(A { x: None, d: D(5) })), d: D(4) };
    let _ = &a;
    println!("end");
}
