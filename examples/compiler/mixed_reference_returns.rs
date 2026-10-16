struct Storage {
    value: usize,
    cells: [i32; 2],
}

fn shared<'a, 'b>(data: &'a mut Storage, flag: &'b bool) -> &'a usize {
    &data.value
}

fn unique<'a, 'b>(data: &'a mut Storage, flag: &'b bool) -> &'a mut usize {
    &mut data.value
}

fn relay<'a, 'b>(data: &'a mut Storage, flag: &'b bool, depth: i32) -> &'a usize {
    if depth == 0 {
        shared(data, flag)
    } else {
        relay(data, flag, depth - 1)
    }
}

fn modify<'a, 'b>(value: &'a i32, other: &'b mut i32) -> &'a i32 {
    *other = 9;
    value
}

fn main() {
    let mut data = Storage {
        value: 4294967296usize,
        cells: [3, 7],
    };
    let flag = true;
    let result = unique(&mut data, &flag);
    *result = 18446744073709551615usize;
    let value = relay(&mut data, &flag, 3);
    let wide_ok = *value == 18446744073709551615usize;
    let mut other = 1;
    let small = modify(&data.cells[1], &mut other);
    if wide_ok && *small == 7 && other == 9 {
        println!("mixed reference returns compiled correctly");
    }
}
