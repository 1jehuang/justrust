// stock: accept
// category: excluded
fn first(bytes: &[u16]) -> u16 { bytes[0] }
fn main() {
    let storage = [7u16; 1];
    let _value = first(&storage);
}
