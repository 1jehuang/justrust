fn rhs() -> i32 { println!("value"); 8 }
fn index() -> usize { println!("index"); 1 }
fn mutate(s: &mut [i32]) {
    s[index()] = rhs();
    s[index()] += rhs();
}
fn main() {
    let mut a = [1, 2, 3];
    mutate(&mut a);
    println!("{} {} {}", a[0], a[1], a[2]);
}
