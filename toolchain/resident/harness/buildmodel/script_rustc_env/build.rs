fn main() {
    println!("cargo:rerun-if-changed=value.txt");
    let value = std::fs::read_to_string("value.txt").unwrap();
    println!("cargo:rustc-env=FROM_SCRIPT={}", value.trim());
}
