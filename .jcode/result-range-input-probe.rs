fn parse(s: &str) -> Result<u32, u8> { if s.is_empty() { Err(1u8) } else { Ok(5u32) } }
fn main() {
    let doubled: Result<Vec<u32>, u8> = (0i32..2i32).map(|_| parse("5")).collect();
    println!("{:?}", doubled.unwrap());
}
