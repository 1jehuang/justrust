// stock: accept
// category: excluded
fn identity<'a>(bytes: &'a [u8]) -> &'a [u8] { bytes }
fn main() {
    let storage = [7u8; 1];
    let bytes: &[u8] = identity(&storage);
    let _value = bytes[0];
}
