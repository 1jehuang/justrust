struct Words {
    narrow: i8,
    wide: u64,
}
fn mark(counter: &mut i32, value: bool) -> bool {
    *counter = *counter + 1;
    value
}
fn combine(words: &mut Words) -> u64 {
    words.narrow = words.narrow ^ 127i8;
    words.wide = (words.wide | 1u64) & 18446744073709551615u64;
    words.wide ^ 1u64
}
fn main() {
    let mut count = 0;
    let eager = mark(&mut count, false) & mark(&mut count, true);
    let other = mark(&mut count, true) | mark(&mut count, false);
    let different = mark(&mut count, true) ^ mark(&mut count, false);
    let mut words = Words {
        narrow: -1i8,
        wide: 9223372036854775808u64,
    };
    let result = combine(&mut words);
    if eager {
        return;
    }
    if !other {
        return;
    }
    if !different {
        return;
    }
    if count != 6 {
        return;
    }
    if words.narrow != -128i8 {
        return;
    }
    if result != 9223372036854775808u64 {
        return;
    }
    if (1u8 | 2u8 ^ 3u8 & 4u8) != 3u8 {
        return;
    }
    if (1u8 + 2u8 & 7u8) != 3u8 {
        return;
    }
    println!("bitwise operations compiled correctly");
}
