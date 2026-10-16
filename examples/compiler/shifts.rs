struct Bits {
    signed: i8,
    wide: u64,
}
fn tick(count: &mut i32, value: u64) -> u64 {
    *count = *count + 1;
    value
}
fn shifted(bits: &mut Bits, left: u64, right: i16) -> u64 {
    bits.signed = bits.signed >> right;
    bits.wide = bits.wide << left;
    bits.wide
}
fn main() {
    let mut bits = Bits {
        signed: -128i8,
        wide: 1u64,
    };
    let result = shifted(&mut bits, 63u64, 7i16);
    let mut count = 0;
    let once = tick(&mut count, 1u64) << tick(&mut count, 3u64);
    let truncated = 127i8 << 1u16;
    let known = 1u8 << (1i16 + 2i16);
    if result != 9223372036854775808u64 {
        return;
    }
    if bits.signed != -1i8 {
        return;
    }
    if once != 8u64 || count != 2 {
        return;
    }
    if truncated != -2i8 || known != 8u8 {
        return;
    }
    if (result >> 63i8) != 1u64 {
        return;
    }
    if ((1u8 as u64) << 63u8) != result {
        return;
    }
    println!("checked shifts compiled correctly");
}
