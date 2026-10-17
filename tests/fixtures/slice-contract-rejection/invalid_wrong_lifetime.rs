// stock: reject
// category: lifetime
fn first(bytes: &[u8]) -> u8 { bytes[0] }
fn main() {
    let bytes: &[u8];
    {
        let storage = [7u8; 1];
        bytes = &storage;
    }
    let _value = first(bytes);
}
