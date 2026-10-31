fn unused_borrow_error() { let original = String::from("moved"); let moved = original; drop(original); drop(moved); }
fn main() {}
