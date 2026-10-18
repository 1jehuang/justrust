struct D { v: i32 }
impl Drop for D {
    fn drop(&mut self) {
        if self.v == 2 { println!("drop lhs"); }
        if self.v == 3 { println!("drop rhs"); }
        if self.v == 5 { println!("drop let"); }
    }
}
fn mark(x: i32) -> i32 { println!("rhs eval"); x }
fn main() {
    if (D { v: 2 }).v == 2 && mark((D { v: 3 }).v) == 3 { println!("and body"); }
    if (D { v: 2 }).v == 0 || mark((D { v: 3 }).v) == 3 { println!("or body"); }
    let b = (D { v: 2 }).v == 2 && (D { v: 3 }).v == 3;
    if b { println!("after let true"); }
    let c = (D { v: 5 }).v == 5;
    if c { println!("after plain true"); }
}
