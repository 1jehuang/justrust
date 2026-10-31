macro_rules! define_unused { () => { fn unused_macro_error() -> u32 { "macro type error" } }; }
define_unused!();
fn main() {}
