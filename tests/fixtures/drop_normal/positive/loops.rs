#![allow(unused_variables)]
struct D { id: i32 }
impl Drop for D {
    fn drop(&mut self) {
        if self.id == 1 { println!("drop 1"); }
        if self.id == 2 { println!("drop 2"); }
        if self.id == 3 { println!("drop 3"); }
    }
}
fn main() {
    let root = D { id: 3 };
    let mut index = 0;
    while index < 2 {
        let iteration = D { id: 1 };
        index = index + 1;
        if index == 1 { println!("continue"); continue; }
        println!("break");
        break;
    }
    'outer: loop {
        let outer = D { id: 1 };
        loop {
            let inner = D { id: 2 };
            println!("labeled break");
            break 'outer;
        }
    }
    println!("done");
}
