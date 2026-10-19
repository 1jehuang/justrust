use std::fmt;
struct Item { value: i32 }
impl fmt::Display for Item {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "item={}", self.value)
    }
}
fn index() -> usize { println!("index"); 1 }
fn later() -> i32 { println!("later"); 7 }
fn main() {
    let mut a = [3, 5, 8];
    let s: &[i32] = &a;
    println!("{} {}", s[index()], later());
    println!("{}", s[0]);
    let m: &mut [i32] = &mut a;
    println!("{}", m[2]);
    m[2] = 13;
    println!("{}", m[index()]);
    let items = [Item { value: 21 }, Item { value: 34 }];
    let borrowed: &[Item] = &items;
    println!("{}", borrowed[0]);
    println!("{}", items[0]);
}
