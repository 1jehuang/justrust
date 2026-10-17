// stock: reject
// category: type
fn main() {
    let storage = [7u8; 1];
    let bytes: [u8] = storage;
    let _ = bytes.len();
}
