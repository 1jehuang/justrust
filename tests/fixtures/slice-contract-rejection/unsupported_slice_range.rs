// stock: accept
// category: excluded
fn suffix_first(bytes: &[u8]) -> u8 {
    let suffix: &[u8] = &bytes[1..];
    suffix[0]
}
fn main() {
    let storage = [7u8, 9u8];
    let _value = suffix_first(&storage);
}
