//! Built by rustc alone the attribute changes nothing a caller can see: every call runs.

use std::sync::atomic::{AtomicU32, Ordering};

static CALLS: AtomicU32 = AtomicU32::new(0);

#[ra_jit::cache]
fn fib(n: u64) -> u64 {
    if n < 2 { n } else { fib(n - 1) + fib(n - 2) }
}

/// Counted, to see that rustc's build calls it every time.
#[ra_jit::cache]
pub(crate) fn counted(mut n: u32, label: &str) -> String {
    CALLS.fetch_add(1, Ordering::Relaxed);
    n += 1;
    format!("{label}{n}")
}

#[ra_jit::cache]
fn largest<T>(items: &[T]) -> Option<T>
where
    T: Ord + Clone + serde::Serialize + serde::de::DeserializeOwned,
{
    items.iter().max().cloned()
}

#[ra_jit::cache]
fn parsed(text: String) -> Result<u32, String> {
    let n: u32 = text.trim().parse().map_err(|e| format!("{e}"))?;
    if n == 0 {
        return Err("zero".into());
    }
    Ok(n * 2)
}

#[ra_jit::cache]
fn nothing() {}

#[derive(serde::Serialize)]
struct Grid;

impl Grid {
    #[ra_jit::cache]
    fn cells(this: &Self, side: u32) -> u32 {
        let _ = this;
        side * side
    }
}

#[test]
fn every_call_runs() {
    assert_eq!(fib(20), 6765);
    assert_eq!(counted(1, "n"), "n2");
    assert_eq!(counted(1, "n"), "n2");
    assert_eq!(CALLS.load(Ordering::Relaxed), 2);
    assert_eq!(largest(&[3, 9, 2]), Some(9));
    assert_eq!(largest::<u8>(&[]), None);
    assert_eq!(parsed(" 21 ".into()), Ok(42));
    assert_eq!(parsed("0".into()), Err("zero".into()));
    assert!(parsed("x".into()).is_err());
    nothing();
    assert_eq!(Grid::cells(&Grid, 7), 49);
}
