fn main() {
    let a: String = String::from("alpha");
    let b: String = String::from("beta");
    let v = vec![&a, &b];
    for s in &v {
        println!("{}", s);
    }
    println!("{:?} {} {}", v, v.len(), v[1].len());
}
