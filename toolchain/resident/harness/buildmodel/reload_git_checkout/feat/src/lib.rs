pub fn voice() -> &'static str {
    if cfg!(feature = "loud") { "LOUD" } else { "soft" }
}
