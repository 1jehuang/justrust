struct W<T: Ord> { v: T, next: Option<Box<W<T>>> }
fn main() { let w = W { v: 1, next: Some(Box::new(W { v: 2, next: None })) }; println!("{} {}", w.v, w.next.map(|n| n.v).unwrap_or(0)); }
