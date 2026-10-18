fn f<T: Drop>(x: &mut T) { <T as Drop>::drop(x); }
fn main() {}
