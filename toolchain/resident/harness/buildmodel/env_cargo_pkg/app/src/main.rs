fn main() {
    println!("{} {} {} {}.{}.{} {:?}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"), env!("CARGO_PKG_AUTHORS"),
        env!("CARGO_PKG_VERSION_MAJOR"), env!("CARGO_PKG_VERSION_MINOR"), env!("CARGO_PKG_VERSION_PATCH"), env!("CARGO_PKG_VERSION_PRE"));
    println!("{} | {} | {} | {} | {}", env!("CARGO_PKG_DESCRIPTION"), env!("CARGO_PKG_HOMEPAGE"), env!("CARGO_PKG_REPOSITORY"),
        env!("CARGO_PKG_LICENSE"), env!("CARGO_PKG_RUST_VERSION"));
    println!("{} {} {}", env!("CARGO_CRATE_NAME"), env!("CARGO_BIN_NAME"), option_env!("CARGO_PRIMARY_PACKAGE").is_some());
    let dir = env!("CARGO_MANIFEST_DIR");
    println!("manifest dir is this one: {}", std::path::Path::new(dir).join("Cargo.toml").exists() && dir.ends_with(env!("TAIL")));
    println!("out dir: {}", env!("OUT_DIR").contains("/build/"));
    println!("dep: {}", dep::about());
}
