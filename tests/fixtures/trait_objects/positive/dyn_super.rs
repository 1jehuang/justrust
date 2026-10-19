trait Named {
    fn id(&self) -> u32;
}
trait Animal: Named {
    fn legs(&self) -> u32;
    fn score(&self) -> u32 {
        self.id() * 100 + self.legs()
    }
}
struct Dog;
struct Bird {
    tag: u32,
}
impl Named for Dog {
    fn id(&self) -> u32 {
        1
    }
}
impl Animal for Dog {
    fn legs(&self) -> u32 {
        4
    }
}
impl Named for Bird {
    fn id(&self) -> u32 {
        self.tag
    }
}
impl Animal for Bird {
    fn legs(&self) -> u32 {
        2
    }
}
fn describe(a: &dyn Animal) -> u32 {
    a.id() + a.score()
}
fn erase<T: Animal>(x: &T) -> u32 {
    let a: &dyn Animal = x;
    describe(a)
}
fn main() {
    let dog = Dog;
    let bird = Bird { tag: 7 };
    if describe(&dog) == 105 && erase(&bird) == 709 {
        println!("supertrait methods through the vtable");
    }
}
