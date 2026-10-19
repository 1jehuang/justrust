//! Slice element parameters retain canonical binder identities during call inference.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
use std::process::Command;

#[test]
fn generic_slice_calls_preserve_element_constraints() {
    let dir = std::env::temp_dir().join(format!("jr-generic-slice-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let source = dir.join("main.rs");
    std::fs::write(&source, "fn identity<'a,T>(s:&'a[T])->&'a[T]{s} fn forward<'a,T>(s:&'a[T])->&'a[T]{identity(s)} unsafe fn raw<'a,T>(p:*const T,n:usize)->&'a[T]{panic!(\"unused stub\")} fn proxy<'a,T>(p:*const T,n:usize)->&'a[T]{unsafe{raw(p,n)}} fn main(){let a=[3u8,4];let s=forward(&a);println!(\"{}\",s.len());}").unwrap();
    let stock = dir.join("stock");
    let output = Command::new("rustc")
        .args(["--edition=2024", "-Awarnings"])
        .arg(&source)
        .arg("-o")
        .arg(&stock)
        .output()
        .unwrap();
    assert!(output.status.success(), "stock: {output:?}");
    let expected = Command::new(stock).output().unwrap();
    for workers in [1, 8] {
        let executable = dir.join(format!("native-{workers}"));
        let checked = Command::new(env!("CARGO_BIN_EXE_jr-rust"))
            .arg(&source)
            .args(["--check", "--workers"])
            .arg(workers.to_string())
            .output()
            .unwrap();
        assert!(
            checked.status.success(),
            "CHECK workers{workers}: {checked:?}"
        );
        assert!(!executable.exists());
        let built = Command::new(env!("CARGO_BIN_EXE_jr-rust"))
            .arg(&source)
            .arg("--workers")
            .arg(workers.to_string())
            .arg("-o")
            .arg(&executable)
            .output()
            .unwrap();
        assert!(built.status.success(), "BUILD workers{workers}: {built:?}");
        let actual = Command::new(executable).output().unwrap();
        assert_eq!(actual.status.code(), expected.status.code());
        assert_eq!(actual.stdout, expected.stdout);
    }
    std::fs::write(
        &source,
        "fn require<T>(s:&[T],v:T){}fn main(){let a=[1u8];require(&a,true);}",
    )
    .unwrap();
    let rejected = Command::new("rustc")
        .args(["--edition=2024", "--emit=metadata"])
        .arg(&source)
        .arg("-o")
        .arg(dir.join("bad.rmeta"))
        .output()
        .unwrap();
    assert!(!rejected.status.success());
    for workers in [1, 8] {
        let rejected = Command::new(env!("CARGO_BIN_EXE_jr-rust"))
            .arg(&source)
            .args(["--check", "--workers"])
            .arg(workers.to_string())
            .output()
            .unwrap();
        assert!(
            !rejected.status.success(),
            "native accepted conflicting slice element"
        );
    }
    let _ = std::fs::remove_dir_all(dir);
}
