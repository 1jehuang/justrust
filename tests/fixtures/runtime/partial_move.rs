struct D { v: i32 }
impl Drop for D { fn drop(&mut self) { println!("drop {}", self.v); } }
struct Q { a: D, b: D }
fn take(d: D) { println!("took {}", d.v); }
fn main() {
    let q = Q { a: D { v: 1 }, b: D { v: 2 } };
    take(q.a);
    println!("end");
}
