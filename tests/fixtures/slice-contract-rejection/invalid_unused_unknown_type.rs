// stock: reject
// category: unused-invalid-body
fn unused(bytes: &[u8]) {
    let _value: MissingType = bytes[0];
}
fn main() {}
