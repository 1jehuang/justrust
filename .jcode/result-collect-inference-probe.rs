fn parse(s: &str) -> Result<u32, u8> { if s.is_empty() { Err(1u8) } else { Ok(5u32) } }
fn main() {
    let doubled: Result<Vec<u32>, u8> = ["5", "6"].iter().map(|s| parse(s).map(|n| n * 2)).collect();
    println!("{:?}", doubled.unwrap());
}
