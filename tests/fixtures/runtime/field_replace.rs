struct D { v: i32 }
impl Drop for D { fn drop(&mut self) { println!("drop {}", self.v); } }
struct Holder { d: D }
fn replace(h: &mut Holder) { h.d = D { v: 9 }; }
fn main() {
    let mut h = Holder { d: D { v: 1 } };
    replace(&mut h);
    println!("replaced {}", h.d.v);
}
