struct Storage {
    byte: u8,
    wide: u64,
}

fn narrow(value: i64) -> i8 {
    value as i8
}

fn extend(value: i8) -> u64 {
    value as u64
}

fn next(counter: &mut usize) -> i16 {
    *counter = *counter + 1usize;
    511i16
}

fn projected(storage: &mut Storage) -> u64 {
    storage.byte as u64
}

fn main() {
    let mut counter = 0usize;
    let byte = next(&mut counter) as u8;
    let mut storage = Storage {
        byte,
        wide: 18446744073709551615u64,
    };
    let snapshot = projected(&mut storage);
    storage.byte = 7u8;
    let block = ({
        let local = 300i32;
        local
    }) as u8;
    if counter == 1usize
        && byte == 255u8
        && narrow(255i64) == -1i8
        && extend(-1i8) == 18446744073709551615u64
        && storage.wide as i64 == -1i64
        && snapshot == 255u64
        && storage.byte == 7u8
        && (200 + 100) as u8 == 44u8
        && block == 44u8
        && (255u16 as i8 as u16) == 65535u16
    {
        println!("integer casts compiled correctly");
    }
}
