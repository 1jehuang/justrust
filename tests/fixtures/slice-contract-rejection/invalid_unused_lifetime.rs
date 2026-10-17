// stock: reject
// category: unused-invalid-body
fn unused() {
    let bytes: &[u8];
    {
        let storage = [7u8; 1];
        bytes = &storage;
    }
    let _value = bytes[0];
}
fn main() {}
