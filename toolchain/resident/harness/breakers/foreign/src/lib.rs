//! A crate of declarations alone, which rustc builds no code for.

unsafe extern "C" {
    pub fn abs(value: i32) -> i32;
    #[link_name = "labs"]
    pub fn long_abs(value: i64) -> i64;
}

/// Defines a `macro_rules!` macro where it is called, as a crate's logging macro is declared once
/// at the top of each file that logs.
#[macro_export]
macro_rules! define_scaled {
    ($name:ident, $by:expr) => {
        macro_rules! $name {
            ($value:expr) => {
                $value * $by
            };
        }
    };
}
