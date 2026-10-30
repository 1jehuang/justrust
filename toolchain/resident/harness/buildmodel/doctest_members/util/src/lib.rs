//! ```
//! assert_eq!(util::halve(8), 4);
//! ```

/// ```
/// assert_eq!(util::halve(9), 4);
/// ```
///
/// ```should_panic
/// util::halve(u32::MAX - util::halve(0)).checked_add(u32::MAX).unwrap();
/// ```
pub fn halve(n: u32) -> u32 {
    n / 2
}

/// ```
/// let mut seen = Vec::new();
/// util::each(3, |it| seen.push(it));
/// assert_eq!(seen, [0, 1, 2]);
/// ```
pub fn each(n: u32, mut f: impl FnMut(u32)) {
    for it in 0..n {
        f(it);
    }
}
