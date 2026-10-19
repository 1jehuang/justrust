trait Counter {
    fn bump(&mut self);
    fn get(&self) -> i32;
    fn value(&self) -> &i32;
}
struct Count {
    n: i32,
}
impl Counter for Count {
    fn bump(&mut self) {
        self.n += 1;
    }
    fn get(&self) -> i32 {
        self.n
    }
    fn value(&self) -> &i32 {
        &self.n
    }
}
fn test(c: &mut dyn Counter) -> i32 {
    c.bump();
    let first = c.get();
    let borrowed = c.value();
    let second = *borrowed;
    c.bump();
    first + second + c.get()
}
fn main() {
    let mut c = Count { n: 1 };
    if test(&mut c) == 7 && c.n == 3 {
        println!("shared methods reborrow mutable objects");
    }
}
