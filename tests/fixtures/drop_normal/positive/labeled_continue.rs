#![allow(unused_variables)]
struct D { id: i32 }
impl Drop for D {
    fn drop(&mut self) {
        if self.id == 1 { println!("drop 1"); }
        if self.id == 2 { println!("drop 2"); }
    }
}
fn main() {
    let mut index = 0;
    'outer: loop {
        let outer = D { id: 1 };
        index = index + 1;
        loop {
            let inner = D { id: 2 };
            if index == 1 { println!("labeled continue"); continue 'outer; }
            println!("labeled break");
            break 'outer;
        }
    }
    println!("done");
}
