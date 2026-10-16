mod storage {
    pub struct Data {
        cells: ([i32; 2], usize),
    }

    pub fn make() -> Data {
        Data {
            cells: ([3, 7], 4294967296usize),
        }
    }

    pub fn wide(data: &mut Data) -> &mut usize {
        &mut data.cells.1
    }

    pub fn second(data: &Data) -> &i32 {
        &data.cells.0[1]
    }
}

fn relay(data: &mut storage::Data, remaining: i32) -> &mut usize {
    if remaining == 0 {
        storage::wide(data)
    } else {
        relay(data, remaining - 1)
    }
}

fn main() {
    let mut data = storage::make();
    let result = relay(&mut data, 3);
    *result = 18446744073709551615usize;
    let wide_ok = *result == 18446744073709551615usize;
    let value = storage::second(&data);
    if wide_ok && *value == 7 {
        println!("reference fields compiled correctly");
    }
}
