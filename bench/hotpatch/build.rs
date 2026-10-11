// Embeds the LD_PRELOAD loader (loader.c) into jr-hot.
use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=loader.c");
    let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("loader.so");
    let ok = Command::new("cc")
        .args(["-shared", "-fPIC", "-O2", "-Wall", "-o"])
        .arg(&out)
        .arg("loader.c")
        .status()
        .expect("cc")
        .success();
    assert!(ok, "building loader.so failed");
}
