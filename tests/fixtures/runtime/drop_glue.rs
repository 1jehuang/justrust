struct D { v: i32 }
impl Drop for D { fn drop(&mut self) { println!("drop {}", self.v); } }
struct P { a: D, b: D, n: i32 }
impl Drop for P { fn drop(&mut self) { println!("drop P {}", self.n); } }
enum E { A(D), B(i32, D), C }
fn make(x: i32) -> E { if x == 0 { E::A(D { v: 10 }) } else if x == 1 { E::B(5, D { v: 11 }) } else { E::C } }
fn main() {
    let p = P { a: D { v: 1 }, b: D { v: 2 }, n: 7 };
    let t = (D { v: 3 }, 4, D { v: 5 });
    let arr = [D { v: 20 }, D { v: 21 }];
    let e0 = make(0);
    let e1 = make(1);
    let e2 = make(2);
    println!("{} {}", p.a.v, t.1);
}
