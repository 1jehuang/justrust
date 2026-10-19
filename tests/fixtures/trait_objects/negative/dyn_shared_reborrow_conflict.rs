trait Counter {
    fn bump(&mut self);
    fn value(&self) -> &i32;
}
struct Count {
    n: i32,
}
impl Counter for Count {
    fn bump(&mut self) {
        self.n += 1;
    }
    fn value(&self) -> &i32 {
        &self.n
    }
}
fn test(c: &mut dyn Counter) -> i32 {
    let borrowed = c.value();
    c.bump();
    *borrowed
}
fn main() {
    let mut c = Count { n: 1 };
    let _ = test(&mut c);
}
