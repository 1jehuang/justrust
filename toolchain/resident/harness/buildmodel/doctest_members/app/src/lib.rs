/// ```
/// assert_eq!(app::quarter(17), 4);
/// println!("{}", app::quarter(100));
/// ```
///
/// ```
/// assert_eq!(app::quarter(4), 1);
/// ```
pub fn quarter(n: u32) -> u32 {
    util::halve(util::halve(n))
}

/// ```no_run
/// app::total(u32::MAX);
/// ```
///
/// ```
/// assert_eq!(app::total(4), 6);
/// ```
pub fn total(n: u32) -> u32 {
    let mut sum = 0;
    util::each(n, |it| sum += it);
    sum
}
