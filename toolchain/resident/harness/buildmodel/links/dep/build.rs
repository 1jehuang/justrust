fn main() {
    println!("cargo:answer=42");
    println!("cargo:rerun-if-changed=build.rs");
}
