struct Numbers {
    tiny: i8,
    small: i16,
    signed: i64,
    word: isize,
    byte: u8,
    half: u16,
    unsigned: u32,
    wide: u64,
}

fn step(x: i8) -> i8 {
    x + 1i8
}

fn signed_div(x: i64, y: i64) -> i64 {
    x / y
}

fn storage(n: &mut Numbers) -> &mut u64 {
    &mut n.wide
}

fn main() {
    let mut n = Numbers {
        tiny: -(128i8),
        small: -32_768i16,
        signed: -9_223_372_036_854_775_808i64,
        word: -9223372036854775808isize,
        byte: 0xffu8,
        half: 0o177777u16,
        unsigned: 0b11111111111111111111111111111111u32,
        wide: 18_446_744_073_709_551_615u64,
    };
    let p = storage(&mut n);
    *p = 9_223_372_036_854_775_808u64;
    let a = [255u8; 2];
    let index = 1usize;
    let selected = match step(n.tiny) {
        -127i8 => true,
        _ => false,
    };
    if selected
        && n.small == -32768i16
        && n.signed < -1i64
        && n.word < 0isize
        && n.byte == 255u8
        && n.half == 65535u16
        && n.unsigned == 4294967295u32
        && n.wide == 9223372036854775808u64
        && a[index] == 255u8
        && signed_div(-7i64, 3i64) == -2i64
        && !0i32 == -1i32
    {
        println!("integer types compiled correctly");
    }
}
