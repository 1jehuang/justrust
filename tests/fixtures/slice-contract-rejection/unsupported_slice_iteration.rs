// stock: accept
// category: excluded
fn sum(bytes: &[u8]) -> u32 {
    let mut total = 0u32;
    for byte in bytes { total = total + *byte as u32; }
    total
}
fn main() {
    let storage = [7u8; 1];
    let _value = sum(&storage);
}
