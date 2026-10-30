#[inline(never)]
pub fn add(a: u8, b: u8) -> u8 {
    a + b
}

/// Generic, so its code is made in the crate that calls it.
pub fn add_generic<T: std::ops::Add<Output = T>>(a: T, b: T) -> T {
    a + b
}

#[inline]
pub fn add_inline(a: u8, b: u8) -> u8 {
    a + b
}

pub fn asserts() -> bool {
    let mut ran = false;
    debug_assert!({
        ran = true;
        true
    });
    ran
}

pub fn cfg_debug() -> bool {
    cfg!(debug_assertions)
}

pub fn opt_level() -> &'static str {
    env!("BUILD_OPT")
}
