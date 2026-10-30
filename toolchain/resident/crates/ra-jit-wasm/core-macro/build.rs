//! Copies the modules of `dioxus-core-macro` that `#[component]` and `Props` are in, for
//! `src/lib.rs` to include.

use std::{env, fs, path::PathBuf};

const VERSION: &str = "0.7.10";

fn main() {
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("core_macro");
    let cargo = env::var_os("CARGO_HOME").map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from(env::var_os("HOME").expect("no home to find cargo's registry in"))
            .join(".cargo")
    });
    // The dependency on the crate is what has cargo download it.
    let registry = cargo.join("registry/src");
    let source = fs::read_dir(&registry)
        .unwrap_or_else(|e| panic!("{}: {e}", registry.display()))
        .filter_map(|it| Some(it.ok()?.path().join(format!("dioxus-core-macro-{VERSION}/src"))))
        .find(|it| it.is_dir())
        .expect("the source of dioxus-core-macro is not in cargo's registry");
    fs::create_dir_all(out.join("props")).unwrap();
    for file in ["component.rs", "utils.rs", "props/mod.rs"] {
        let text = fs::read_to_string(source.join(file)).unwrap();
        // `syn::parse` is of the tokens of rustc, which there are none of outside a proc macro.
        fs::write(out.join(file), text.replace("syn::parse(", "syn::parse2(")).unwrap();
    }
    let path = |file: &str| format!("#[path = {:?}] pub mod", out.join(file));
    let modules = format!(
        "{} component;\n{} utils;\n{} props;\n",
        path("component.rs"),
        path("utils.rs"),
        path("props/mod.rs")
    );
    fs::write(out.join("mod.rs"), modules).unwrap();
    println!("cargo::rerun-if-changed=build.rs");
}
