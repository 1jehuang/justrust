fn main() {
    println!("cargo:rerun-if-env-changed=BM_LEVEL");
    let level = std::env::var("BM_LEVEL").unwrap_or_else(|_| "unset".to_owned());
    println!("cargo:rustc-env=LEVEL={level}");
    println!("cargo::rustc-check-cfg=cfg(loud)");
    if level == "loud" {
        println!("cargo:rustc-cfg=loud");
    }
}
