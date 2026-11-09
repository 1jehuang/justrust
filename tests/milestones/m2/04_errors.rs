use std::fmt;
use std::num::ParseIntError;
#[derive(Debug)]
enum AppError { Parse(ParseIntError), Negative(i64), Empty }
impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self { AppError::Parse(e) => write!(f, "parse: {e}"), AppError::Negative(n) => write!(f, "negative: {n}"), AppError::Empty => write!(f, "empty") }
    }
}
impl std::error::Error for AppError {}
impl From<ParseIntError> for AppError { fn from(e: ParseIntError) -> Self { AppError::Parse(e) } }
fn parse_pos(s: &str) -> Result<i64, AppError> {
    let s = s.trim(); if s.is_empty() { return Err(AppError::Empty); }
    let n: i64 = s.parse()?; if n < 0 { Err(AppError::Negative(n)) } else { Ok(n) }
}
fn sum_line(line: &str) -> Result<i64, AppError> { line.split(',').map(parse_pos).sum() }
fn boxed(line: &str) -> Result<i64, Box<dyn std::error::Error>> { let v = sum_line(line)?; if v > 100 { return Err("too big".into()); } Ok(v) }
fn first_char_upper(s: &str) -> Option<char> { let c = s.chars().next()?; Some(c.to_ascii_uppercase()) }
fn main() {
    for l in ["1,2,3", "4, x", "5,-6", "7,,8", "50,60"] {
        match sum_line(l) { Ok(v) => println!("ok {v}"), Err(e) => println!("err {e}") }
        match boxed(l) { Ok(v) => println!("  boxed ok {v}"), Err(e) => println!("  boxed err {e}") }
    }
    println!("{:?} {:?}", first_char_upper("rust"), first_char_upper(""));
    let r: Result<Vec<i32>, _> = "1 2 3".split(' ').map(str::parse::<i32>).collect();
    println!("{:?}", r);
    let r2: Result<Vec<i32>, _> = "1 b 3".split(' ').map(str::parse::<i32>).collect();
    println!("{}", r2.is_err());
    let v = parse_pos("12").map(|x| x * 2).unwrap_or_default();
    let w = parse_pos("-1").ok().and_then(|x| x.checked_mul(2)).unwrap_or(-99);
    println!("{v} {w} {:?}", parse_pos("z").map_err(|e| e.to_string()));
}
