struct D { id: i32 }
impl Drop for D {
    fn drop(&mut self) {
        if self.id == 1 { println!("drop 1"); }
        if self.id == 2 { println!("drop 2"); }
        if self.id == 3 { println!("drop 3"); }
        if self.id == 4 { println!("drop 4"); }
    }
}
fn params(first: D, second: D) {
    let local = D { id: 3 };
    println!("params body");
}
fn main() {
    {
        let first = D { id: 1 };
        let second = D { id: 2 };
        {
            let third = D { id: 3 };
            let fourth = D { id: 4 };
            println!("inner body");
        }
        println!("outer body");
    }
    params(D { id: 1 }, D { id: 2 });
    println!("done");
}
