fn main() {
    for (name, var) in [("BUILD_PROFILE", "PROFILE"), ("BUILD_OPT", "OPT_LEVEL"), ("BUILD_DEBUG", "DEBUG")] {
        println!("cargo:rustc-env={name}={}", std::env::var(var).unwrap());
    }
}
