struct D { v: i32 }
impl Drop for D { fn drop(&mut self) { println!("drop {}", self.v); } }
fn main() {
    let b = Box::new(5);
    let x = *b.as_ref() + 1;
    println!("{}", x);
    let mut d = Box::new(D { v: 3 });
    d.as_mut().v = 4;
    let e = Box::new(D { v: 8 });
    let inner = *e;
    println!("inner {}", inner.v);
    let pair = Box::new((Box::new(D { v: 1 }), 2));
    let y = pair.as_ref().1;
    println!("{}", y);
}
