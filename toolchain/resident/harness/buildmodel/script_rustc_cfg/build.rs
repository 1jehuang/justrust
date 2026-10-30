fn main() {
    println!("cargo:rerun-if-changed=flag");
    println!("cargo::rustc-check-cfg=cfg(flagged)");
    if std::fs::read_to_string("flag").is_ok_and(|it| it.trim() == "on") {
        println!("cargo:rustc-cfg=flagged");
    }
}
