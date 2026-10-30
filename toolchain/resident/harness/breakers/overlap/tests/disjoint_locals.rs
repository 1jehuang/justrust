//! 200 frames of `tables` or `temporaries` in a stack of 2 MB: one table each fits, eight do not.

#[test]
fn deep() {
    let thread = std::thread::Builder::new().stack_size(2 << 20);
    let sum = thread.spawn(|| overlap::tables(0, 200)).unwrap().join().unwrap();
    assert_eq!(sum, (0..=200u32).map(|n| (n as u8 ^ (n % 8 * 31) as u8) as u64).sum());
}

#[test]
fn deep_temporaries() {
    let thread = std::thread::Builder::new().stack_size(2 << 20);
    let sum = thread.spawn(|| overlap::temporaries(0, 200)).unwrap().join().unwrap();
    assert_eq!(sum, (0..=200u32).map(|n| (n as u8 ^ (n % 8 * 31) as u8) as u64).sum());
}
