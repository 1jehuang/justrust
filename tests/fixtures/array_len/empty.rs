fn main() {
    let a: [i32; 0] = [];
    let n = a.len();
    println!("{}", n);
    let mut b: [i32; 0] = [];
    let r = &mut b;
    let n = r.len();
    println!("{}", n);
    let n = (&a).len();
    println!("{}", n);
    let n = (&mut b).len();
    println!("{}", n);
}
