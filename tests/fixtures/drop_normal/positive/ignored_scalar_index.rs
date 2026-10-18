struct D { id: i32 }
impl Drop for D {
    fn drop(&mut self) {
        if self.id == 1 { println!("drop first"); }
        if self.id == 2 { println!("drop second"); }
        if self.id == 3 { println!("drop nested"); }
        if self.id == 4 { println!("drop iteration"); }
    }
}
fn work(index: usize) {
    let arr = [10, 20];
    let first = D { id: 1 };
    let _ = arr[index];
    let second = D { id: 2 };
    {
        let nested = D { id: 3 };
        let _ = arr[index];
        println!("nested body");
    }
    println!("after nested");
    let mut count = 0;
    while count < 2 {
        let iteration = D { id: 4 };
        let _ = arr[index];
        println!("iteration body");
        count = count + 1;
    }
    println!("outer body");
}
fn main() { work(1usize); }
