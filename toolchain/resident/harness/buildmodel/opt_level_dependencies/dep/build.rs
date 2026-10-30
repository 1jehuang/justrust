fn main() {
    println!("cargo:rustc-env=BUILD_OPT={}", std::env::var("OPT_LEVEL").unwrap());
}
