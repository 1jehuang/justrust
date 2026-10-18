struct D { v: i32 }
impl Drop for D {
    fn drop(&mut self) {
        if self.v == 1 { println!("drop cond"); }
        if self.v == 2 { println!("drop lhs"); }
        if self.v == 3 { println!("drop rhs"); }
        if self.v == 4 { println!("drop while"); }
    }
}
fn main() {
    if (D { v: 1 }).v == 1 { println!("then body"); } else { println!("else body"); }
    if (D { v: 1 }).v == 0 { println!("then body"); } else { println!("else body"); }
    if (D { v: 2 }).v == 2 && (D { v: 3 }).v == 3 { println!("and body"); }
    if (D { v: 2 }).v == 0 && (D { v: 3 }).v == 3 { println!("and body"); } else { println!("short body"); }
    let mut n: i32 = 0;
    while (D { v: 4 }).v + n < 6 { println!("while body"); n = n + 1; }
    println!("done");
}
