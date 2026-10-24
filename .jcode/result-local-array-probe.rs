fn parse(s: &str) -> Result<u32, u8> { if s.is_empty() { Err(1u8) } else { Ok(5u32) } }
fn main() {
    let input = ["5", "6"];
    let doubled: Vec<Result<u32, u8>> = input.iter().map(|s: &&str| parse(*s)).collect();
    println!("{}", doubled.len());
}
