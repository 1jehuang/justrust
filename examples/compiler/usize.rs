fn index() -> usize {
    1
}

fn relay(value: usize) -> usize {
    value
}

fn main() {
    let high: usize = 18446744073709551615;
    let mut values: [usize; 3] = [4294967296, high, 7];
    let saved = values[index()];
    values[index()] = relay(4294967296);
    if saved == high && values[index()] == 4294967296 && !high == 0 {
        println!("usize and runtime indexing compiled correctly");
    }
}
