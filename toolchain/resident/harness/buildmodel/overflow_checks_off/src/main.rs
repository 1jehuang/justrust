use std::panic;

#[inline(never)]
fn add(a: u8, b: u8) -> u8 {
    a + b
}

fn shift(a: u32, by: u32) -> u32 {
    a << by
}

fn overflow() -> String {
    let n = std::env::args().count() as u8;
    match panic::catch_unwind(|| add(254 + n, n)) {
        Ok(sum) => format!("wrapped to {sum}"),
        Err(_) => "panicked".to_owned(),
    }
}

fn main() {
    panic::set_hook(Box::new(|_| {}));
    println!("cfg(debug_assertions): {}", cfg!(debug_assertions));
    let mut ran = false;
    debug_assert!({
        ran = true;
        true
    });
    println!("debug_assert! ran: {ran}");
    println!("overflow: {}", overflow());
    let by = 31 + std::env::args().count() as u32;
    println!("shift: {:?}", panic::catch_unwind(|| shift(1, by)).ok());
    println!("profile: {} opt-level {} debug {}", env!("BUILD_PROFILE"), env!("BUILD_OPT"), env!("BUILD_DEBUG"));
}
