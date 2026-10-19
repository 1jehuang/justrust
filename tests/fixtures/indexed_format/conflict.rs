fn change(s: &mut [i32]) -> i32 { s[0] = 9; 0 }
fn main() {
    let mut a = [1, 2];
    println!("{} {}", a[0], change(&mut a));
}
