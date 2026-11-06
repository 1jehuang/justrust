#[derive(Debug, Clone, PartialEq)]
struct Point { x: i32, y: i32 }
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dir { N, E, S, W }
fn main() {
    let p = Point { x: 1, y: 2 };
    let q = p.clone();
    println!("{:?} {}", q, p == q);
    let d = Dir::E;
    println!("{:?}", d);
    let v = vec![Dir::N, Dir::S, Dir::W];
    println!("{:?}", v);
}
