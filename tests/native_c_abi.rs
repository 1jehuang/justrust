#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};
#[path = "support/stock_rustc.rs"]
mod stock_rustc;

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
fn captured_native_link_ignores_ambient_path_driver_specs_and_library_environment() {
    use std::os::unix::fs::PermissionsExt;
    let scratch = Scratch::new();
    let source = scratch.file("ambient.rs");
    fs::write(&source, "unsafe extern \"C\"{fn labs(x:isize)->isize;}fn main(){assert_eq!(unsafe{labs(-42)},42);println!(\"closed native link\");}").unwrap();
    let poison = scratch.file("poison");
    fs::create_dir(&poison).unwrap();
    let marker = scratch.file("ambient-tool-was-executed");
    for name in ["cc", "gcc", "ld", "collect2", "as"] {
        let file = poison.join(name);
        fs::write(
            &file,
            format!(
                "#!/bin/sh\nprintf poisoned > '{}'\nexit 73\n",
                marker.display()
            ),
        )
        .unwrap();
        fs::set_permissions(file, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let mut baseline = None;
    for workers in [1, 8] {
        for hostile in [false, true] {
            let executable = scratch.file(&format!("ambient-{workers}-{hostile}"));
            let mut command = Command::new(cli());
            command
                .arg(&source)
                .args([
                    "--workers",
                    &workers.to_string(),
                    "--native-library",
                    "c",
                    "-o",
                ])
                .arg(&executable);
            if hostile {
                command
                    .env("PATH", &poison)
                    .env("COMPILER_PATH", &poison)
                    .env("GCC_EXEC_PREFIX", &poison)
                    .env("LIBRARY_PATH", &poison)
                    .env("LD_LIBRARY_PATH", &poison)
                    .env("LD_RUN_PATH", &poison)
                    .env("GNUTARGET", "not-an-object-format")
                    .env("LDEMULATION", "not-an-emulation")
                    .env("COLLECT_GCC_OPTIONS", "-specs=/not-a-captured-input");
            }
            let output = command.output().unwrap();
            assert!(
                output.status.success(),
                "{command:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let bytes = fs::read(&executable).unwrap();
            if let Some(expected) = &baseline {
                assert_eq!(&bytes, expected);
            } else {
                baseline = Some(bytes);
            }
            assert_eq!(
                run(&mut Command::new(executable)).stdout,
                b"closed native link\n"
            );
            assert!(
                !marker.exists(),
                "ambient driver/linker execution escaped the capsule"
            );
        }
    }
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
fn narrow_scalar_array_imports_exports_and_callbacks_match_stock() {
    let scratch = Scratch::new();
    let source = scratch.file("packed.rs");
    let c_source = scratch.file("packed.c");
    let object = scratch.file("packed.o");
    fs::write(
        &c_source,
        r#"
#include <stdint.h>
typedef struct { uint16_t values[3]; } Tiny;
typedef struct { uint8_t bytes[3]; uint16_t halves[3]; float floats[3]; } Mixed;
extern Mixed rust_packed(Mixed);
Tiny c_tiny(Tiny x){x.values[1]+=7;return x;}
Mixed c_mixed(Mixed x){x.bytes[2]+=3;x.halves[1]+=11;x.floats[0]+=0.5f;return x;}
Mixed c_callback(Mixed x){return rust_packed(x);}
"#,
    )
    .unwrap();
    fs::write(&source,r#"
#[repr(C)]#[derive(Clone,Copy)]pub struct Tiny{values:[u16;3]}
#[repr(C)]#[derive(Clone,Copy)]pub struct Mixed{bytes:[u8;3],halves:[u16;3],floats:[f32;3]}
unsafe extern "C"{fn c_tiny(x:Tiny)->Tiny;fn c_mixed(x:Mixed)->Mixed;fn c_callback(x:Mixed)->Mixed;}
#[unsafe(no_mangle)]pub extern "C" fn rust_packed(mut x:Mixed)->Mixed{x.bytes[1]+=2u8;x.halves[2]+=13u16;x.floats[2]+=0.25f32;x}
fn main(){
    let tiny=unsafe{c_tiny(Tiny{values:[257u16,513u16,769u16]})};assert_eq!(tiny.values[0],257u16);assert_eq!(tiny.values[1],520u16);assert_eq!(tiny.values[2],769u16);
    let source=Mixed{bytes:[11u8,22u8,33u8],halves:[257u16,513u16,769u16],floats:[1.5f32,2.25f32,3.5f32]};
    let got=unsafe{c_mixed(source)};assert_eq!(got.bytes[0],11u8);assert_eq!(got.bytes[1],22u8);assert_eq!(got.bytes[2],36u8);assert_eq!(got.halves[0],257u16);assert_eq!(got.halves[1],524u16);assert_eq!(got.halves[2],769u16);assert_eq!(got.floats[0],2.0f32);assert_eq!(got.floats[1],2.25f32);assert_eq!(got.floats[2],3.5f32);
    let got=unsafe{c_callback(got)};assert_eq!(got.bytes[1],24u8);assert_eq!(got.halves[2],782u16);assert_eq!(got.floats[2],3.75f32);assert_eq!(got.halves[1],524u16);assert_eq!(got.floats[1],2.25f32);
    println!("packed C ABI");
}"#).unwrap();
    run(Command::new("cc")
        .args(["-std=c11", "-O2", "-fno-pie", "-c"])
        .arg(&c_source)
        .arg("-o")
        .arg(&object));
    let stock = scratch.file("stock");
    run(Command::new(stock_rustc::stock_rustc())
        .args(["--edition=2024", "-Cpanic=abort", "-Clink-arg=-no-pie"])
        .arg(&source)
        .arg(format!("-Clink-arg={}", object.display()))
        .arg("-o")
        .arg(&stock));
    let expected = run(&mut Command::new(stock));
    assert_eq!(expected.stdout, b"packed C ABI\n");
    for workers in [1, 8] {
        run(Command::new(cli())
            .arg(&source)
            .args([
                "--check",
                "--workers",
                &workers.to_string(),
                "--native-object",
            ])
            .arg(&object));
        let native = scratch.file(&format!("packed-{workers}"));
        run(Command::new(cli())
            .arg(&source)
            .args(["--workers", &workers.to_string(), "--native-object"])
            .arg(&object)
            .arg("-o")
            .arg(&native));
        let actual = run(&mut Command::new(native));
        assert_eq!(actual.stdout, expected.stdout);
        assert_eq!(actual.stderr, expected.stderr);
    }
}

#[test]
fn wide_private_frame_addresses_survive_c_import_export_and_callbacks() {
    let scratch = Scratch::new();
    let source = scratch.file("wide-frame.rs");
    let c_source = scratch.file("wide-frame.c");
    let object = scratch.file("wide-frame.o");
    fs::write(
        &c_source,
        r#"
#include <stdint.h>
extern unsigned __int128 rust_frame_wide(uint64_t, unsigned __int128);
extern uint64_t rust_frame_even(uint64_t, uint64_t);
uint64_t c_frame_address(uintptr_t address) {
    const unsigned __int128 *p = (const unsigned __int128 *)address;
    return (address & 15u) + (uint64_t)(*p & 65535u);
}
unsigned __int128 c_frame_reenter(uint64_t tag, unsigned __int128 value) {
    return rust_frame_wide(tag, value) + 5u;
}
uint64_t c_frame_even(uint64_t a, uint64_t b) {
    return rust_frame_even(a, b);
}
"#,
    )
    .unwrap();
    fs::write(
        &source,
        r#"
unsafe extern "C" {
    fn c_frame_address(address:usize)->u64;
    fn c_frame_reenter(tag:u64,value:u128)->u128;
    fn c_frame_even(a:u64,b:u64)->u64;
}
fn aligned(value:&u128){assert_eq!(value as *const u128 as usize%16usize,0usize);}
fn private_wide(tag:u64,value:u128)->u128{
    aligned(&value);let local=value+tag as u128;aligned(&local);local
}
#[unsafe(no_mangle)]
pub extern "C" fn rust_frame_wide(tag:u64,value:u128)->u128{
    aligned(&value);assert_eq!(tag,7u64);
    let array=[value,value+1u128];aligned(&array[0]);aligned(&array[1]);
    assert_eq!(unsafe{c_frame_address(&array[1] as *const u128 as usize)},10u64);
    let got=private_wide(tag,value);aligned(&got);got
}
#[unsafe(no_mangle)]
pub extern "C" fn rust_frame_even(a:u64,b:u64)->u64{
    assert_eq!(a,11u64);assert_eq!(b,13u64);let local=17u128;aligned(&local);
    unsafe{c_frame_address(&local as *const u128 as usize)}
}
fn main(){
    let before=19u64;let value=18446744073709551625u128;aligned(&value);
    assert_eq!(unsafe{c_frame_address(&value as *const u128 as usize)},9u64);
    let got=unsafe{c_frame_reenter(7u64,value)};aligned(&got);assert_eq!(got,value+12u128);
    assert_eq!(unsafe{c_frame_even(11u64,13u64)},17u64);assert_eq!(before,19u64);
    println!("wide C frame boundaries");
}
"#,
    )
    .unwrap();
    run(Command::new("cc")
        .args(["-std=c11", "-O2", "-fno-pie", "-c"])
        .arg(&c_source)
        .arg("-o")
        .arg(&object));
    let stock = scratch.file("stock");
    run(Command::new(stock_rustc::stock_rustc())
        .args(["--edition=2024", "-Cpanic=abort", "-Clink-arg=-no-pie"])
        .arg(&source)
        .arg(format!("-Clink-arg={}", object.display()))
        .arg("-o")
        .arg(&stock));
    let expected = run(&mut Command::new(stock));
    assert_eq!(expected.stdout, b"wide C frame boundaries\n");
    for workers in [1, 8] {
        run(Command::new(cli())
            .arg(&source)
            .args([
                "--check",
                "--workers",
                &workers.to_string(),
                "--native-object",
            ])
            .arg(&object));
        let native = scratch.file(&format!("wide-frame-{workers}"));
        run(Command::new(cli())
            .arg(&source)
            .args(["--workers", &workers.to_string(), "--native-object"])
            .arg(&object)
            .arg("-o")
            .arg(&native));
        let actual = run(&mut Command::new(native));
        assert_eq!(actual.stdout, expected.stdout);
        assert_eq!(actual.stderr, expected.stderr);
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
    run(Command::new(stock_rustc::stock_rustc())
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
