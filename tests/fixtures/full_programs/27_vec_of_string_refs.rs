fn main() {
    let a: String = String::from("alpha");
    let b: String = String::from("beta");
    let v = vec![&a, &b];
    let mut total = 0;
    for s in &v {
        println!("{} {}", s, s.len());
        total += s.len();
    }
    println!("{:?} {} {}", v, v.len(), total);
}
