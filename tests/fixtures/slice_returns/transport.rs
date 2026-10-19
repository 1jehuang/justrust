fn same<T>(s: &[T]) -> &[T] { s }
fn same_mut<T>(s: &mut [T]) -> &mut [T] { s }
fn tail(s: &[i32]) -> &[i32] { &s[1..] }
struct Item { value: i32 }
fn main() {
    let mut values = [1, 2, 3];
    let r = same_mut::<i32>(&mut values);
    r[1] = 9;
    let read = same::<i32>(&values);
    let first = read[0];
    let second = read[1];
    let end = tail(read);
    let end_first = end[0];
    println!("{} {} {}", first, second, end_first);
    let items = [Item { value: 7 }, Item { value: 8 }];
    let borrowed = same::<Item>(&items);
    let count = borrowed.len();
    println!("{}", count);
    let mut items_mut = [Item { value: 4 }];
    let borrowed_mut = same_mut::<Item>(&mut items_mut);
    let count_mut = borrowed_mut.len();
    println!("{}", count_mut);
    let empty: [i32; 0] = [];
    let count_empty = same::<i32>(&empty).len();
    println!("{}", count_empty);
}
