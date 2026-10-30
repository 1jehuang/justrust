//! bitvec names a macro `safe!` and calls it in an `extern` block. `safe` is a keyword there,
//! before `fn` and `static`, and was taken for one before `!` too: the item did not parse and
//! the functions that used it had errors.

macro_rules! safe {
    ($($item:tt)*) => { $($item)* };
}

unsafe extern "C" {
    safe!(fn abs(value: i32) -> i32;);
}

pub fn run() -> i32 {
    unsafe { abs(-42) }
}
