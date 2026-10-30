fn main() {
    println!("cargo:rustc-cfg=scripted");
    println!("cargo:rustc-env=BM_FROM_SCRIPT=first");
}
