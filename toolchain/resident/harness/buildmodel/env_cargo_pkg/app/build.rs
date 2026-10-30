fn main() {
    let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    println!("cargo:rustc-env=TAIL={}", std::path::Path::new(&dir).file_name().unwrap().to_str().unwrap());
}
