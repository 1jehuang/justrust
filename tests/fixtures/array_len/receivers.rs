struct Item { x: i32 }
impl Drop for Item { fn drop(&mut self) { println!("drop {}", (*self).x); } }
struct Holder { a: [i32; 2] }
fn make() -> [i32; 3] { println!("make"); [4, 5, 6] }
fn shared(a: &[i32; 3]) -> usize { a.len() }
fn mutable(a: &mut [i32; 3]) -> usize { let n = a.len(); (*a)[0] = 9; n }
fn main() {
    let mut a = [1, 2, 3];
    let n = a.len(); println!("{}", n);
    let n = shared(&a); println!("{}", n);
    let n = mutable(&mut a); println!("{}", n);
    println!("{}", a[0]);
    let h = Holder { a: [7, 8] };
    let n = h.a.len(); println!("{}", n);
    let n = make().len(); println!("{}", n);
    let values = [Item { x: 10 }, Item { x: 11 }];
    let n = values.len(); println!("{}", n);
    println!("{}", values[0].x);
    let n = [4, 5].len(); println!("{}", n);
}
