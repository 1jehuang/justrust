#[cfg(from_flags)]
fn which() -> &'static str {
    "from the flags"
}

#[cfg(not(from_flags))]
fn which() -> &'static str {
    "without"
}

fn main() {
    println!("{} {} {:?}", which(), cfg!(from_flags), option_env!("FROM_CONFIG"));
}
