fn cls(c: char) -> u32 {
    match c {
        '\0' => 0,
        '\u{1}'..='\u{1f}' => 1,
        'A'..='Z' => 2,
        'a'..='z' => 3,
        '0'..='9' => 4,
        '\u{80}'..'\u{E000}' => 5,
        '\u{10FFFF}' => 6,
        _ => 7,
    }
}
fn sgn(n: i8) -> u32 {
    match n {
        i8::MIN => 0,
        -127..=-1 => 1,
        0 => 2,
        1..i8::MAX => 3,
        i8::MAX => 4,
    }
}
fn wide(n: i128) -> u32 {
    match n {
        ..-1000 => 0,
        -1000..=1000 => 1,
        1001.. => 2,
    }
}
fn nested(v: (u8, char, bool)) -> u32 {
    match v {
        (0..=9, 'a'..='f', true) => 1,
        (10.., 'a'..='f', _) => 2,
        (_, 'g'.., false) => 3,
        _ => 4,
    }
}
fn guarded(n: u16) -> u32 {
    match n {
        0..=49 => 1,
        0..=99 => 2,
        _ => 3,
    }
}
fn check(ok: bool, label: u32) {
    if ok { println!("ok"); } else { if label == 0 { println!("fail0"); } else { println!("fail"); } }
}
fn main() {
    check(cls('\0') == 0 && cls('\u{1f}') == 1 && cls(' ') == 7 && cls('A') == 2 && cls('Z') == 2 && cls('[') == 7, 1);
    check(cls('a') == 3 && cls('z') == 3 && cls('0') == 4 && cls('9') == 4 && cls('\u{7f}') == 7, 2);
    check(cls('\u{80}') == 5 && cls('\u{D7FF}') == 5 && cls('\u{E000}') == 7 && cls('\u{10FFFF}') == 6, 3);
    check(sgn(-128) == 0 && sgn(-127) == 1 && sgn(-1) == 1 && sgn(0) == 2 && sgn(1) == 3 && sgn(126) == 3 && sgn(127) == 4, 4);
    check(wide(-1001) == 0 && wide(-1000) == 1 && wide(1000) == 1 && wide(1001) == 2, 5);
    check(nested((5, 'c', true)) == 1 && nested((5, 'c', false)) == 4 && nested((200, 'f', false)) == 2 && nested((1, 'z', false)) == 3 && nested((1, 'z', true)) == 4, 6);
    check(guarded(4) == 1 && guarded(55) == 2 && guarded(100) == 3, 7);
}
