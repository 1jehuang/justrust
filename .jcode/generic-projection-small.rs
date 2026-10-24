fn visit<T>(values: &[T]) { let mut mapped = values.iter().map(|_x| 7i32); match mapped.next() { Some(n) => assert_eq!(n, 7i32), None => panic!() } }
fn main() { let data = [1, 2]; visit(&data); }
