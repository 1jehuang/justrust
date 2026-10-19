struct Item { value: i64 }
struct Holder { values: [i32; 4] }
fn tail<'a, T>(values: &'a mut [T]) -> &'a mut [T] { &mut values[1..] }
fn ranges(values: &mut [i32]) {
    let a = &mut values[1..3];
    a[0] += 10;
    let b = &mut values[2..];
    b[0] += 20;
    let c = &mut values[..2];
    c[0] += 30;
    let d = &mut values[..];
    d[3] += 40;
    let e = &mut values[1..=2];
    e[0] += 50;
    let f = &mut values[..=0];
    f[0] += 60;
    let empty = &mut values[2..2];
    println!("{}", empty.len());
    let shared = &values[1..3];
    println!("{} {}", shared[0], shared[1]);
}
fn main() {
    let mut holder = Holder { values: [1, 2, 3, 4] };
    ranges(&mut holder.values);
    println!("{} {} {} {}", holder.values[0], holder.values[1], holder.values[2], holder.values[3]);
    let result = tail(&mut holder.values);
    result[0] += 1;
    let nested = &mut result[1..][..1];
    nested[0] += 2;
    println!("{} {}", holder.values[1], holder.values[2]);
    let mut items = [Item { value: 7 }, Item { value: 8 }];
    let view = tail(&mut items);
    println!("{}", view.len());
}
