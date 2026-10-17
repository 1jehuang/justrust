// stock: reject
// category: unused-invalid-body
fn unused(bytes: &[u8]) -> u8 {
    missing_byte + bytes[0]
}
fn main() {}
