fn classify(c: char) -> i32 {
    match c {
        'a' => 1,
        'b'..='z' => 2,
        '0'..='9' => 3,
        _ => 0,
    }
}
fn bucket(n: i32) -> i32 {
    match n {
        ..=-1 => 0,
        0 => 1,
        1..10 => 2,
        10.. => 3,
    }
}
fn byte(b: u8) -> i32 {
    match b {
        0..=127 => 1,
        128..=255 => 2,
    }
}
fn pair(p: (char, i32)) -> i32 {
    match p {
        ('x', 1..=3) => 1,
        (_, 4..) => 2,
        _ => 3,
    }
}
fn main() {
    if classify('a') == 1 && classify('q') == 2 && classify('7') == 3 && classify('!') == 0 { println!("classify ok"); }
    if bucket(-5) == 0 && bucket(0) == 1 && bucket(9) == 2 && bucket(10) == 3 && bucket(2147483647) == 3 { println!("bucket ok"); }
    if byte(5) == 1 && byte(200) == 2 { println!("byte ok"); }
    if pair(('x', 2)) == 1 && pair(('y', 9)) == 2 && pair(('x', 0)) == 3 { println!("pair ok"); }
}
