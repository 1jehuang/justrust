#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "jr-c-abi-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn file(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn run(command: &mut Command) -> std::process::Output {
    let output = Command::new("timeout")
        .arg("120")
        .arg(command.get_program())
        .args(command.get_args())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{command:?}: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}
fn cli() -> PathBuf {
    std::env::var_os("JR_G6_CLI")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_jr-rust")))
}
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/c_abi")
        .join(name)
}
#[test]
fn optimized_imports_keep_real_external_call_bodies_at_workers_1_8() {
    let scratch = Scratch::new();
    let source = scratch.file("optimized.rs");
    let c_source = scratch.file("add.c");
    let object = scratch.file("add.o");
    fs::write(&source, "unsafe extern \"C\"{fn native_add(a:i32,b:i32)->i32;} fn invoke()->i32{unsafe{native_add(20,22)}} #[unsafe(no_mangle)] extern \"C\" fn rust_add(a:i32,b:i32)->i32{a+b} fn main(){assert_eq!(invoke(),42);assert_eq!(rust_add(19,23),42);println!(\"optimized C passed\");}").unwrap();
    fs::write(&c_source, "int native_add(int a,int b){return a+b;}\n").unwrap();
    run(Command::new("cc")
        .args(["-c", "-O2"])
        .arg(&c_source)
        .arg("-o")
        .arg(&object));
    for mode in ["adaptive", "final"] {
        for workers in [1, 8] {
            let executable = scratch.file(&format!("{mode}-{workers}"));
            run(Command::new(cli())
                .arg(&source)
                .args([
                    "--opt",
                    mode,
                    "--workers",
                    &workers.to_string(),
                    "--native-object",
                ])
                .arg(&object)
                .arg("-o")
                .arg(&executable));
            assert_eq!(
                run(&mut Command::new(executable)).stdout,
                b"optimized C passed\n"
            );
        }
    }
}

#[test]
fn real_c_imports_exports_callbacks_and_native_library_match_stock_at_workers_1_8() {
    let scratch = Scratch::new();
    let c = scratch.file("oracle.o");
    run(Command::new("cc")
        .args(["-std=c11", "-O2", "-fno-pie", "-c"])
        .arg(fixture("oracle.c"))
        .arg("-o")
        .arg(&c));
    let stock = scratch.file("stock");
    run(Command::new("rustc")
        .args(["--edition=2024", "-Cpanic=abort", "-Clink-arg=-no-pie"])
        .arg(fixture("program.rs"))
        .arg(format!("-Clink-arg={}", c.display()))
        .arg("-o")
        .arg(&stock));
    let expected = run(&mut Command::new(stock));
    assert_eq!(
        expected.stdout,
        b"C ABI roundtrip passed\nNative lifecycle passed\n"
    );
    for workers in [1, 8] {
        let native = scratch.file(&format!("native-{workers}"));
        run(Command::new(cli())
            .arg(fixture("program.rs"))
            .args(["--workers", &workers.to_string(), "--native-object"])
            .arg(&c)
            .args(["--native-library", "c", "-o"])
            .arg(&native));
        let actual = run(&mut Command::new(native));
        assert_eq!(actual.stdout, expected.stdout);
        assert_eq!(actual.stderr, expected.stderr);
        let object = scratch.file(&format!("native-{workers}.o"));
        run(Command::new(cli())
            .arg(fixture("program.rs"))
            .args(["--workers", &workers.to_string(), "--emit-object", "-o"])
            .arg(&object));
        let from_c = scratch.file(&format!("from-c-{workers}"));
        run(Command::new("cc")
            .args(["-no-pie", "-DG6_C_MAIN"])
            .arg(fixture("oracle.c"))
            .arg(&object)
            .arg("-o")
            .arg(&from_c));
        let from_c = run(&mut Command::new(from_c));
        assert_eq!(from_c.stdout, b"C entry passed\n");
    }
}
