// stock: accept
// category: excluded
fn length<T>(bytes: &[T]) -> usize { bytes.len() }
fn main() {
    let storage = [7u8; 1];
    let _length = length(&storage);
}
