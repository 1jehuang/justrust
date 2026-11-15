//! Bounded primitive structural Debug. This is not std::fmt trait dispatch.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/iterator_oracle.rs"]
mod cell_oracle;
#[path = "support/oracle.rs"]
mod oracle;
use std::{fs, path::PathBuf, process::Command};

#[test]
fn cell_debug_covers_supported_scalar_payloads() {
    cell_oracle::stock_native(
        r#"
use std::cell::Cell;
#[derive(Debug)]
struct State { count: Cell<u32>, marked: Cell<bool> }
fn main() {
    let state = State { count: Cell::new(3u32), marked: Cell::new(false) };
    println!("{:?}", state);
    state.count.set(4);
    state.marked.set(true);
    println!("{:#?}", state);
    println!("{:?} {:?}", Cell::new(-128i8), Cell::new(255u8));
    println!("{:?} {:?}", Cell::new(-32768i16), Cell::new(65535u16));
    println!("{:?} {:?}", Cell::new(-2147483648i32), Cell::new(4294967295u32));
    println!("{:?} {:?}", Cell::new(-9223372036854775808i64), Cell::new(18446744073709551615u64));
    println!("{:?} {:?}", Cell::new(-170141183460469231731687303715884105728i128), Cell::new(340282366920938463463374607431768211455u128));
    println!("{:?} {:?}", Cell::new(-7isize), Cell::new(9usize));
    println!("{:?} {:?}", Cell::new(1.25f32), Cell::new(-0.0f64));
    println!("{:?} {:?}", Cell::new('λ'), Cell::new('\n'));
    println!("{:?} {:?}", Cell::new(false), Cell::new(true));
}
"#,
    );
    for source in [
        "use std::cell::Cell; fn unused(value:Cell<String>){println!(\"{:?}\",value);} fn main(){}",
        "use std::cell::Cell; fn unused<T>(value:Cell<T>){println!(\"{:?}\",value);} fn main(){}",
    ] {
        cell_oracle::stock_and_native_reject(source);
    }
    // Formatting support must not admit generic Cell schemas into source code.
    let unsupported = "use std::cell::Cell; fn unused<T:Copy+std::fmt::Debug>(value:&Cell<T>){println!(\"{:?}\",value);} fn main(){}";
    for workers in [1, 8] {
        let failure = justrust_engine::compiler::compile(unsupported.into(), workers, true)
            .err()
            .expect("generic Cell payload remains explicitly unsupported");
        let diagnostic = failure.to_string();
        assert!(
            diagnostic.contains("unsupported") && diagnostic.contains("Cell"),
            "{diagnostic}"
        );
    }
}

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let root = std::env::var_os("JCODE_SCRATCH_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join(format!("jr-debug-streams-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        Self(root)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn debug_streams_integer_widths_and_explicit_boundaries_match_stock() {
    let scratch = Scratch::new();
    let mut source = String::from("fn main(){");
    for (ty, min, max) in [
        ("i8", "-128", "127"),
        ("u8", "0", "255"),
        ("i16", "-32768", "32767"),
        ("u16", "0", "65535"),
        ("i32", "-2147483648", "2147483647"),
        ("u32", "0", "4294967295"),
        ("i64", "-9223372036854775808", "9223372036854775807"),
        ("u64", "0", "18446744073709551615"),
        ("isize", "-9223372036854775808", "9223372036854775807"),
        ("usize", "0", "18446744073709551615"),
        (
            "i128",
            "-170141183460469231731687303715884105728",
            "170141183460469231731687303715884105727",
        ),
        ("u128", "0", "340282366920938463463374607431768211455"),
    ] {
        source.push_str(&format!("println!(\"{{:?}} {{:?}}\",{min}{ty},{max}{ty});"));
    }
    source.push_str(r#"print!("{:?}",((),[true,false]));println!("{:?}",[1,2]);eprint!("{:?}",(3,));eprintln!("{:?}",[(),()]);}"#);
    let mut cases = vec![("live", source, None)];
    // char/str/float Debug and float Display are now library-formatted and
    // must match stock output exactly.
    for (name, expr, mode) in [
        ("char_debug", "'λ'", ":?"),
        ("str_debug", "\"quote\\\"\\n\"", ":?"),
        ("float_debug", "1.25f64", ":?"),
        ("float_display", "-0.0f64", ""),
    ] {
        cases.push((
            name,
            format!("fn main(){{println!(\"{{{mode}}}\",{expr});}}"),
            None::<&str>,
        ));
    }
    for (name, source, boundary) in cases {
        let path = scratch.0.join(format!("{name}.rs"));
        fs::write(&path, &source).unwrap();
        let stock_image = scratch.0.join(format!("stock-{name}"));
        let stock = Command::new("rustc")
            .env_remove("RUSTC_BOOTSTRAP")
            .args(["--edition=2024", "-Cpanic=abort", "-Awarnings"])
            .arg(&path)
            .arg("-o")
            .arg(&stock_image)
            .output()
            .expect("stock rustc required: run this differential test locally");
        assert!(stock.status.success(), "{name}: {stock:?}");
        let expected = Command::new(&stock_image).output().unwrap();
        assert!(expected.status.success(), "{name}: {expected:?}");
        for workers in [1, 8] {
            for check in [true, false] {
                let image = scratch.0.join(format!("native-{name}-{workers}-{check}"));
                let mut command = Command::new(env!("CARGO_BIN_EXE_jr-rust"));
                command.arg(&path).args(["--workers", &workers.to_string()]);
                if check {
                    command.arg("--check");
                } else {
                    command.arg("-o").arg(&image);
                }
                let actual = command.output().unwrap();
                if let Some(boundary) = boundary {
                    assert!(!actual.status.success(), "{name}: unexpectedly accepted");
                    let diagnostic = String::from_utf8_lossy(&actual.stderr);
                    assert!(diagnostic.contains(boundary), "{name}: {diagnostic}");
                    assert!(!image.exists());
                } else {
                    assert!(actual.status.success(), "{name}: {actual:?}");
                    if check {
                        assert!(!image.exists());
                    } else {
                        let actual = Command::new(&image).output().unwrap();
                        assert_eq!(actual.status.code(), expected.status.code(), "{name}");
                        assert_eq!(actual.stdout, expected.stdout, "{name} stdout");
                        assert_eq!(actual.stderr, expected.stderr, "{name} stderr");
                    }
                }
            }
        }
    }
}

#[test]
fn structural_debug_matches_stock_and_preserves_evaluation() {
    use oracle::{Case, Expect};
    oracle::check(
        "structural-debug",
        &[
            Case {
                name: "scalars",
                source: r#"fn main(){println!("{} {:?} {:?} {:?} {:?}",7, -128i8,340282366920938463463374607431768211455u128,true,());}"#,
                expect: Expect::Run("7 -128 340282366920938463463374607431768211455 true ()\n"),
            },
            Case {
                name: "aggregate",
                source: r#"fn main(){let a=[(1i32,true),(2,false)];let empty:[i32;0]=[];println!("{:?}|{:?}|{:?}|{:?}",a,(3,),((4,()),[5,6]),empty);println!("{:?}",[();2]);}"#,
                expect: Expect::Run(
                    "[(1, true), (2, false)]|(3,)|((4, ()), [5, 6])|[]\n[(), ()]\n",
                ),
            },
            Case {
                name: "evaluate_once",
                source: r#"fn next(n:&mut i32)->(i32,bool){*n+=1;print!("eval{}|",*n);(*n,true)}fn main(){let mut n=0;println!("{:?} {:?} {}",next(&mut n),next(&mut n),n);}"#,
                expect: Expect::Run("eval1|eval2|(1, true) (2, true) 2\n"),
            },
            Case {
                name: "copy_place_remains_usable",
                source: r#"fn main(){let a=[1,2];let t=(3,true);println!("{:?} {:?}",a,t);println!("{:?} {:?}",a,t);}"#,
                expect: Expect::Run("[1, 2] (3, true)\n[1, 2] (3, true)\n"),
            },
            Case {
                name: "tuple_borrow_conflict",
                source: r#"fn main(){let mut a=(1,true);println!("{:?} {:?}",a,{a.0=2;a});}"#,
                expect: Expect::Invalid("E0506"),
            },
            Case {
                name: "array_borrow_conflict",
                source: r#"fn main(){let mut a=[1,2];println!("{:?} {:?}",a,{a[0]=3;a});}"#,
                expect: Expect::Invalid("E0506"),
            },
        ],
    );
}
