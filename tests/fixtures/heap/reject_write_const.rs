fn main() {
    let x = 1u8;
    let p = &x as *const u8;
    unsafe {
        *p = 2;
    }
}
