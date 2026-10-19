struct Rows { values: [i32; 3] }
fn bump(values: &mut [i32]) {
    values[0] += 4;
    values[2] = 9;
}
fn main() {
    let mut rows = Rows { values: [1, 2, 3] };
    let values: &mut [i32] = &mut rows.values;
    bump(values);
    let first = values[0];
    let last = values[2];
    println!("{} {}", first, last);
    let shared: &[i32] = &rows.values;
    let len = shared.len();
    let middle = shared[1];
    println!("{} {}", len, middle);
}
