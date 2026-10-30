pub fn about() -> String {
    format!("{} {} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"), option_env!("CARGO_PRIMARY_PACKAGE").is_some())
}
