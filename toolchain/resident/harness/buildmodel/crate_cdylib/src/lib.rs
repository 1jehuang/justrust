#[no_mangle]
pub extern "C" fn triple(n: u32) -> u32 {
    scaled(n, 3)
}

#[inline(never)]
fn scaled(n: u32, by: u32) -> u32 {
    let parts: Vec<u32> = (0..by).map(|_| n).collect();
    parts.iter().sum()
}
