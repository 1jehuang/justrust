fn id<T>(x: T) -> T {
    x
}

fn main() {
    let a: String = String::from("hello");
    let r = id(&a);
    println!("{} {}", r, r.len());
}
