use std::panic;

#[inline(never)]
fn add(a: u8, b: u8) -> u8 {
    a + b
}

fn tried(f: impl Fn() -> u8 + panic::UnwindSafe) -> String {
    match panic::catch_unwind(f) {
        Ok(sum) => format!("wrapped to {sum}"),
        Err(_) => "panicked".to_owned(),
    }
}

fn main() {
    panic::set_hook(Box::new(|_| {}));
    let n = std::env::args().count() as u8;
    println!("app add: {}", tried(move || add(254 + n, n)));
    println!("dep add: {}", tried(move || dep::add(254 + n, n)));
    println!("dep add_generic: {}", tried(move || dep::add_generic(254 + n, n)));
    println!("dep add_inline: {}", tried(move || dep::add_inline(254 + n, n)));
    println!("app cfg(debug_assertions): {}", cfg!(debug_assertions));
    println!("dep cfg(debug_assertions): {} asserts {}", dep::cfg_debug(), dep::asserts());
    println!("dep opt-level: {}", dep::opt_level());
}
