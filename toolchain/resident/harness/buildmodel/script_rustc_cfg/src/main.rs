#[cfg(flagged)]
fn which() -> &'static str {
    "flagged"
}

#[cfg(not(flagged))]
fn which() -> &'static str {
    "plain"
}

fn main() {
    println!("{} {}", which(), cfg!(flagged));
}
