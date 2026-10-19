fn start() -> usize { println!("start"); 1 }
fn end() -> usize { println!("end"); 3 }
fn main() {
    let mut a = [1, 2, 3, 4];
    println!("before");
    let view = &mut a[start()..end()];
    view[0] = 7;
    let n = view.len();
    println!("{}", n);
    println!("{}", a[1]);
}
