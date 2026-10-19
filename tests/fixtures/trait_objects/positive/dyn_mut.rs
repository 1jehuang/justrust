trait Counter {
    fn bump(&mut self, by: i32);
    fn get(&self) -> i32;
}
struct Up {
    n: i32,
}
struct Down {
    n: i32,
}
impl Counter for Up {
    fn bump(&mut self, by: i32) {
        self.n += by;
    }
    fn get(&self) -> i32 {
        self.n
    }
}
impl Counter for Down {
    fn bump(&mut self, by: i32) {
        self.n -= by;
    }
    fn get(&self) -> i32 {
        self.n
    }
}
fn twice(c: &mut dyn Counter) {
    c.bump(2);
    c.bump(3);
}
fn nested(c: &mut dyn Counter) {
    twice(c);
    twice(c);
}
fn main() {
    let mut up = Up { n: 0 };
    let mut down = Down { n: 100 };
    nested(&mut up);
    twice(&mut down);
    if up.get() == 10 && down.get() == 95 {
        println!("mutable trait objects reborrow");
    }
}
