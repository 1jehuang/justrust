fn parse(s: &str) -> Result<u32, u8> { if s.is_empty() { Err(1u8) } else { Ok(5u32) } }
fn main() {
    let doubled: Result<Vec<u32>, u8> = ["5", "6"].iter().map(|s: &&str| parse(*s).map(|n: u32| n * 2)).collect();
    println!("{:?}", doubled.unwrap());
}
