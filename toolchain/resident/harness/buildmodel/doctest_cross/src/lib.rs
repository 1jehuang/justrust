/// ```
/// assert_eq!(app::arch(), "aarch64");
/// assert_eq!(app::words(), 8);
/// ```
///
/// ```should_panic
/// assert_eq!(app::arch(), "x86_64");
/// ```
///
/// ```ignore-aarch64
/// assert_eq!(app::arch(), "x86_64");
/// ```
///
/// ```ignore-x86_64
/// assert_eq!(app::arch(), "aarch64");
/// ```
pub fn arch() -> &'static str {
    std::env::consts::ARCH
}

pub fn words() -> usize {
    std::mem::size_of::<usize>()
}
