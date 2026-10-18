struct D { id: i32 }
impl Drop for D {
    fn drop(&mut self) {
        if self.id == 1 { println!("drop 1"); }
        if self.id == 2 { println!("drop 2"); }
        if self.id == 3 { println!("drop 3"); }
    }
}
fn identity(value: D) -> D { value }
fn make() -> D {
    let local = D { id: 2 };
    let result = D { id: 1 };
    println!("returning");
    return result;
}
fn main() {
    let returned = make();
    println!("caller owns");
    let moved = identity(returned);
    println!("identity returned");
    let last = D { id: 3 };
    println!("done");
}
