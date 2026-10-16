struct Values {
    word: u64,
    signed: i8,
}
fn rhs(log: &mut i32) -> u64 {
    *log = *log * 10 + 2;
    3u64
}
fn index(log: &mut i32) -> usize {
    *log = *log * 10 + 1;
    0usize
}
fn update(p: &mut u64, count: i16) {
    *p += 4u64;
    *p /= 3u64;
    *p <<= count;
}
fn main() {
    let mut a = [2u64, 10u64];
    let mut log = 0;
    a[index(&mut log)] += rhs(&mut log);
    if log != 21 || a[0] != 5u64 || a[1] != 10u64 {
        return;
    }
    let i = 0usize;
    a[i] /= 5u64;
    a[i] <<= 63u8;
    a[i] >>= 63i16;
    if a[0] != 1u64 {
        return;
    }
    let mut values = Values {
        word: 2u64,
        signed: -128i8,
    };
    update(&mut values.word, 63i16);
    values.signed >>= 7u64;
    if values.word != 0u64 || values.signed != -1i8 {
        return;
    }
    let mut x = 2u64;
    let mut y = 10u64;
    let mut p = &mut x;
    *p += {
        p = &mut y;
        3u64
    };
    if x != 2u64 || y != 13u64 {
        return;
    }
    let mut initialized: i32;
    initialized += {
        initialized = 20;
        2
    };
    if initialized != 22 {
        return;
    }
    let mut flag = false;
    let mut effects = 0;
    flag &= {
        effects += 1;
        true
    };
    flag |= {
        effects += 1;
        true
    };
    flag ^= {
        effects += 1;
        true
    };
    if flag || effects != 3 {
        return;
    }
    println!("compound assignments compiled correctly");
}
