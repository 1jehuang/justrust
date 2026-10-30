pub fn enabled() -> String {
    let mut on = Vec::new();
    if cfg!(feature = "a") { on.push("a"); }
    if cfg!(feature = "b") { on.push("b"); }
    if cfg!(feature = "host") { on.push("host"); }
    on.join("+")
}
