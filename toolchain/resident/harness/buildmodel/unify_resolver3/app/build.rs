fn main() {
    println!("cargo:rustc-env=HOST_SEES={}", shared::enabled());
}
