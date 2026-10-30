use std::{env, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=native.c");
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    if env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        archive_msvc(&out);
    } else {
        archive(&out);
    }
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=native");
}

fn archive(out: &PathBuf) {
    let object = out.join("native.o");
    let cc = env::var("BM_CC").unwrap_or_else(|_| "cc".to_owned());
    assert!(Command::new(&cc).args(["-c", "-fPIC", "native.c", "-o"]).arg(&object).status().unwrap().success());
    let ar = env::var("BM_AR").unwrap_or_else(|_| "ar".to_owned());
    assert!(Command::new(&ar).arg("crs").arg(out.join("libnative.a")).arg(&object).status().unwrap().success());
}

/// The same with the compiler and librarian of MSVC, which the environment of its tools has on the path.
fn archive_msvc(out: &PathBuf) {
    let object = out.join("native.obj");
    let cc = env::var("BM_CC").unwrap_or_else(|_| "cl".to_owned());
    let mut made = std::ffi::OsString::from("/Fo");
    made.push(&object);
    assert!(Command::new(&cc).args(["/nologo", "/c", "native.c"]).arg(made).status().unwrap().success());
    let ar = env::var("BM_AR").unwrap_or_else(|_| "lib".to_owned());
    let mut archive = std::ffi::OsString::from("/OUT:");
    archive.push(out.join("native.lib"));
    assert!(Command::new(&ar).arg("/nologo").arg(archive).arg(&object).status().unwrap().success());
}
