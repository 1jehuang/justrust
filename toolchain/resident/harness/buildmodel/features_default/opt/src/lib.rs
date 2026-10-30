pub fn enabled() -> Vec<&'static str> {
    let mut on = Vec::new();
    if cfg!(feature = "loud") { on.push("loud"); }
    on
}
