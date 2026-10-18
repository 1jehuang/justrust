use std::fmt;

#[derive(Debug)]
enum ParseError {
    Empty,
    BadDigit(char),
    Overflow,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ParseError::Empty => write!(f, "empty input"),
            ParseError::BadDigit(c) => write!(f, "bad digit '{}'", c),
            ParseError::Overflow => write!(f, "overflow"),
        }
    }
}

fn parse_u32(s: &str) -> Result<u32, ParseError> {
    if s.is_empty() {
        return Err(ParseError::Empty);
    }
    let mut n: u32 = 0;
    for c in s.chars() {
        let d = c.to_digit(10).ok_or(ParseError::BadDigit(c))?;
        n = n.checked_mul(10).ok_or(ParseError::Overflow)?;
        n = n.checked_add(d).ok_or(ParseError::Overflow)?;
    }
    Ok(n)
}

fn sum_all(items: &[&str]) -> Result<u32, ParseError> {
    let mut total = 0;
    for it in items {
        total += parse_u32(it)?;
    }
    Ok(total)
}

fn divide(a: i32, b: i32) -> Option<i32> {
    if b == 0 { None } else { Some(a / b) }
}

fn compute(a: i32, b: i32, c: i32) -> Option<i32> {
    let x = divide(a, b)?;
    let y = divide(x, c)?;
    Some(x + y)
}

fn main() {
    for s in ["123", "", "12a", "99999999999", "4294967295"] {
        match parse_u32(s) {
            Ok(n) => println!("{:?} -> {}", s, n),
            Err(e) => println!("{:?} -> error: {}", s, e),
        }
    }
    println!("{:?}", sum_all(&["1", "2", "3"]).map_err(|e| e.to_string()));
    println!("{:?}", sum_all(&["1", "x", "3"]).map_err(|e| e.to_string()));
    println!("{:?}", compute(100, 5, 2));
    println!("{:?}", compute(100, 0, 2));
    let fallback = parse_u32("oops").unwrap_or(7);
    println!("fallback = {}", fallback);
    let doubled: Result<Vec<u32>, ParseError> = ["5", "6"].iter().map(|s| parse_u32(s).map(|n| n * 2)).collect();
    println!("doubled = {:?}", doubled.unwrap());
}
