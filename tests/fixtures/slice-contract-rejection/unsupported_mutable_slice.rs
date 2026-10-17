// stock: accept
// category: excluded
fn update(bytes: &mut [u8]) { bytes[0] = 9u8; }
fn main() {
    let mut storage = [7u8; 1];
    update(&mut storage);
}
