//! Runtime overflow profiles are independent of optimization and cfg assertions.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
use justrust_engine::compiler::{self, source::SourceOptions, ssa::OptMode};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::ExitStatusExt;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Scratch(std::path::PathBuf);
impl Scratch {
    fn new() -> Self {
        let root = std::env::var_os("JCODE_SCRATCH_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let path = root.join(format!(
            "jr-overflow-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn run(&self, name: &str, bytes: &[u8]) -> Output {
        let path = self.0.join(name);
        fs::write(&path, bytes).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        for attempt in 0..100 {
            match Command::new(&path).output() {
                Err(error) if error.raw_os_error() == Some(26) && attempt < 99 => {
                    std::thread::sleep(std::time::Duration::from_millis(1))
                }
                output => return output.unwrap(),
            }
        }
        unreachable!()
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn options(checked: bool, opt: OptMode, prelude: bool) -> SourceOptions {
    SourceOptions {
        overflow_checks: checked,
        opt,
        prelude,
        ..SourceOptions::default()
    }
}
fn differential(source: &str, checked: bool, success: bool) {
    // Isolate primitive semantics from unrelated embedded-library checking.
    // The opaque integration case below retains the default prelude.
    differential_with_prelude(source, checked, success, false);
}
fn differential_with_prelude(source: &str, checked: bool, success: bool, prelude: bool) {
    let scratch = Scratch::new();
    let input = scratch.0.join("input.rs");
    let stock = scratch.0.join("stock");
    fs::write(&input, source).unwrap();
    let compiled = Command::new("rustc")
        .arg(&input)
        .args(["--edition=2024", "-C", "panic=abort", "-C"])
        .arg(format!(
            "overflow-checks={}",
            if checked { "yes" } else { "no" }
        ))
        .arg("-o")
        .arg(&stock)
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "stock compile: {}\n{source}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let expected = Command::new(stock).output().unwrap();
    assert_eq!(
        expected.status.success(),
        success,
        "stock: {expected:?}\n{source}"
    );
    let fixture = &source[..source.len().min(100)];
    for workers in [1, 8] {
        eprintln!("overflow CHECK checked={checked} workers={workers}: {fixture}");
        let checked_ir = compiler::compile_observed_with_options(
            source.into(),
            workers,
            true,
            options(checked, OptMode::Off, prelude),
        )
        .unwrap_or_else(|e| panic!("CHECK {checked} {workers}: {e}\n{source}"));
        assert!(checked_ir.executable.is_none());
    }
    for opt in [OptMode::Off, OptMode::Adaptive, OptMode::Final] {
        for workers in [1, 8] {
            eprintln!(
                "overflow BUILD/run checked={checked} opt={opt:?} workers={workers}: {fixture}"
            );
            let build = compiler::compile_observed_with_options(
                source.into(),
                workers,
                false,
                options(checked, opt, prelude),
            )
            .unwrap_or_else(|e| panic!("BUILD {checked} {opt:?} {workers}: {e}\n{source}"));
            let actual = scratch.run("native", build.executable.as_ref().unwrap());
            assert_eq!(
                actual.status.success(),
                success,
                "{checked} {opt:?} {workers}: {actual:?}\n{source}"
            );
            assert_eq!(
                actual.stdout, expected.stdout,
                "{checked} {opt:?} {workers}"
            );
            if !success {
                assert_eq!(actual.status.signal(), expected.status.signal());
            }
        }
    }
}
#[test]
fn primitive_no_prelude_smoke() {
    differential_with_prelude(
        "fn f(x:u8)->u8{x+1u8}fn main(){println!(\"{}\",f(255u8));}",
        false,
        true,
        false,
    );
}

#[test]
fn all_widths_wrapping_arithmetic_and_masked_shifts() {
    let mut source = String::new();
    let mut calls = String::new();
    for (ty, bits, signed) in [
        ("i8", 8, true),
        ("u8", 8, false),
        ("i16", 16, true),
        ("u16", 16, false),
        ("i32", 32, true),
        ("u32", 32, false),
        ("i64", 64, true),
        ("u64", 64, false),
        ("isize", 64, true),
        ("usize", 64, false),
        ("i128", 128, true),
        ("u128", 128, false),
    ] {
        let max = if signed {
            (1u128 << (bits - 1)) - 1
        } else {
            u128::MAX >> (128 - bits)
        };
        let min = if signed {
            format!("-{}", 1u128 << (bits - 1))
        } else {
            "0".into()
        };
        source.push_str(&format!("fn test_{ty}(max:{ty}, min:{ty}, one:{ty}, shift:i32) {{ if max+one != min {{panic!(\"add\")}} if min-one != max {{panic!(\"sub\")}}"));
        let product = if signed {
            format!("-2{ty}")
        } else {
            "max-one".into()
        };
        source.push_str(&format!("if max*2{ty} != {product} {{panic!(\"mul\")}}"));
        if signed {
            source.push_str("if -min != min {panic!(\"neg\")}");
        }
        source.push_str(&format!("if one << shift != 2{ty} {{panic!(\"shl\")}} if max >> shift != max >> 1u8 {{panic!(\"shr\")}} }}"));
        calls.push_str(&format!(
            "test_{ty}({max}{ty},{min}{ty},1{ty},{}i32);",
            bits + 1
        ));
    }
    source.push_str(&format!("fn main(){{{calls} println!(\"ok\");}}"));
    differential(&source, false, true);
}
#[test]
fn checked_addition_traps_at_each_declared_width() {
    for (ty, bits, signed) in [
        ("i8", 8, true),
        ("u8", 8, false),
        ("i16", 16, true),
        ("u16", 16, false),
        ("i32", 32, true),
        ("u32", 32, false),
        ("i64", 64, true),
        ("u64", 64, false),
        ("isize", 64, true),
        ("usize", 64, false),
        ("i128", 128, true),
        ("u128", 128, false),
    ] {
        let max = if signed {
            (1u128 << (bits - 1)) - 1
        } else {
            u128::MAX >> (128 - bits)
        };
        differential(
            &format!(
                "fn f(x:{ty})->{ty}{{x+1{ty}}}fn main(){{let x=f({max}{ty});println!(\"{{}}\",x);}}"
            ),
            true,
            false,
        );
    }
}

#[test]
fn checked_overflow_and_unconditional_traps() {
    for checked in [false, true] {
        for body in [
            "fn f(x:i8,y:i8)->i8{x/y} fn main(){let x=f(-128i8,-1i8);println!(\"{}\",x);}",
            "fn f(x:i128,y:i128)->i128{x%y} fn main(){let x=f(-170141183460469231731687303715884105728i128,-1i128);println!(\"{}\",x);}",
            "fn f(x:u128,y:u128)->u128{x/y} fn main(){let x=f(4u128,0u128);println!(\"{}\",x);}",
            "fn at(i:usize){let a=[1i32,2];println!(\"{}\",a[i]);}fn main(){at(3usize);}",
            "fn main(){panic!(\"explicit\");}",
        ] {
            differential(body, checked, false);
        }
    }
    differential(
        "fn f(x:u8)->u8{x+1u8} fn main(){println!(\"{}\",f(255u8));}",
        true,
        false,
    );
}
#[test]
fn wide_and_negative_shift_counts_are_masked_at_the_lhs_width() {
    differential(
        "fn f(n:i128){if 1u8<<n != 128u8 {panic!(\"narrow\")} if 1u128<<n != 170141183460469231731687303715884105728u128 {panic!(\"wide\")} if -2i128>>n != -1i128 {panic!(\"signed\")}}fn main(){f(-1i128);f(340282366920938463463374607431768211455u128 as i128);println!(\"ok\");}",
        false,
        true,
    );
}

#[test]
fn opaque_physical_body_uses_the_selected_runtime_policy() {
    let source = "trait Read{fn read(&self)->u8;}struct Number{value:u8}impl Read for Number{fn read(&self)->u8{self.value}}fn make()->impl Read{Number{value:255u8}}fn use_value(add:u8)->u8{let value=make();value.read()+add}fn main(){println!(\"{}\",use_value(1u8));}";
    differential_with_prelude(source, false, true, true);
    differential_with_prelude(source, true, false, true);
}

#[test]
fn unchecked_runtime_count_does_not_use_checked_constant_evaluation() {
    differential(
        "fn f(a:u8,b:u8)->u8{1u8 << (a+b)}fn main(){let x=f(255u8,2u8);println!(\"{}\",x);}",
        false,
        true,
    );
}
#[test]
fn invalid_unused_bodies_and_ctfe_still_fail() {
    for source in [
        "fn bad()->u8{1u8<<8u8}fn main(){}",
        "fn bad()->u8{1u8<<(255u8+2u8)}fn main(){}",
        "fn bad()->u8{true} fn main(){}",
        "const BAD:u8=255u8+1u8;fn main(){}",
        "fn bad(){let x=-1u8;}fn main(){}",
    ] {
        for checked in [false, true] {
            let scratch = Scratch::new();
            let input = scratch.0.join("invalid.rs");
            fs::write(&input, source).unwrap();
            // Stock's metadata-only CHECK does not run the MIR arithmetic_overflow
            // lint for these unused bodies. BUILD rejects them in either profile.
            // Native CHECK intentionally performs these diagnostics eagerly.
            let stock = Command::new("rustc")
                .arg(&input)
                .args(["--edition=2024", "-C"])
                .arg(format!(
                    "overflow-checks={}",
                    if checked { "yes" } else { "no" }
                ))
                .arg("-o")
                .arg(scratch.0.join("invalid.elf"))
                .output()
                .unwrap();
            assert!(
                !stock.status.success(),
                "stock unexpectedly accepted {source}"
            );
            for workers in [1, 8] {
                assert!(
                    compiler::compile_observed_with_options(
                        source.into(),
                        workers,
                        true,
                        options(checked, OptMode::Off, false)
                    )
                    .is_err(),
                    "{source}"
                );
            }
        }
    }
}
