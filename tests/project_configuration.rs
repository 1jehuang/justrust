//! Native CLI admission tests against actual stock Cargo/rustc behavior.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

use std::path::PathBuf;
use std::process::{Command, Output};

struct Project(PathBuf);
impl Project {
    fn new(name: &str, edition: &str, profile: &str, source: &str) -> Self {
        let root = std::env::var_os("JCODE_SCRATCH_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join(format!("jr-configuration-{}-{name}", std::process::id()));
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("Cargo.toml"), format!(
            "[package]\nname = \"configuration_probe\"\nversion = \"0.1.0\"\n{edition}\n{profile}\n"
        )).unwrap();
        std::fs::write(root.join("src/main.rs"), source).unwrap();
        Self(root)
    }
    fn native(&self, args: &[&str], workers: &str) -> Output {
        Command::new(env!("CARGO_BIN_EXE_jr-rust"))
            .args(args)
            .args(["-j", workers])
            .current_dir(&self.0)
            .env("CARGO_TARGET_DIR", self.0.join("native-target"))
            .output()
            .unwrap()
    }
    fn stock(&self, args: &[&str]) -> Output {
        Command::new("cargo")
            .args(args)
            .current_dir(&self.0)
            .env("CARGO_TARGET_DIR", self.0.join("stock-target"))
            .output()
            .expect("stock Cargo is required for differential tests")
    }
}
fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}
fn rejected(output: Output, diagnostic: &str) {
    assert_eq!(output.status.code(), Some(101), "{}", stderr(&output));
    assert!(stderr(&output).contains(diagnostic), "{}", stderr(&output));
    assert!(!stderr(&output).contains("Finished"));
}

#[test]
fn invalid_editions_are_rejected_by_both_clis() {
    let p = Project::new(
        "invalid-edition",
        "edition = \"2099\"",
        "",
        "fn main() {}\n",
    );
    assert!(
        !p.stock(&["metadata", "--no-deps", "--format-version", "1"])
            .status
            .success()
    );
    for workers in ["1", "8"] {
        for command in ["check", "build", "run"] {
            rejected(p.native(&[command], workers), "invalid Rust edition `2099`");
        }
    }
}

#[test]
fn edition_2021_is_admitted_with_stock_semantics_or_explicit_boundaries() {
    // Supported 2021 programs match stock; constructs whose 2021 semantics
    // differ from the implemented 2024 rules reject explicitly.
    let cases = [
        (
            "plain",
            "fn main() { let v = [1u32, 2]; println!(\"{}\", v.iter().sum::<u32>()); }\n",
            None,
        ),
        (
            "pattern-reset",
            "fn main() { let (ref x,) = &(4u32,); println!(\"{}\", x); }\n",
            Some("pre-2024 default binding mode"),
        ),
        (
            "tail-temporary",
            "struct D(u32); impl Drop for D { fn drop(&mut self) { println!(\"drop {}\", self.0); } } impl D { fn v(&self) -> u32 { self.0 } } fn g() -> u32 { let _a = D(1); D(2).v() } fn main() { println!(\"{}\", g()); }\n",
            Some("pre-2024 temporary scope"),
        ),
        (
            "if-let-temporary",
            "struct D(u32); impl Drop for D { fn drop(&mut self) { println!(\"drop {}\", self.0); } } impl D { fn o(&self) -> Option<u32> { Some(self.0) } } fn main() { if let Some(v) = D(2).o() { println!(\"{}\", v); } else { println!(\"none\"); } println!(\"end\"); }\n",
            Some("pre-2024 temporary scope"),
        ),
        (
            "opaque-capture",
            "fn f(x: &u32) -> impl Fn() -> u32 { let y = *x; move || y } fn main() { let v = 3; println!(\"{}\", f(&v)()); }\n",
            Some("pre-2024 `impl Trait`"),
        ),
    ];
    for (name, source, boundary) in cases {
        let p = Project::new(&format!("e2021-{name}"), "edition = \"2021\"", "", source);
        let stock = p.stock(&["run", "--offline", "--quiet"]);
        assert!(stock.status.success(), "{name}: {}", stderr(&stock));
        for workers in ["1", "8"] {
            match boundary {
                None => {
                    assert!(p.native(&["check"], workers).status.success(), "{name}");
                    let native = p.native(&["run"], workers);
                    assert!(native.status.success(), "{name}: {}", stderr(&native));
                    assert_eq!(native.stdout, stock.stdout, "{name}");
                }
                Some(diagnostic) => {
                    for command in ["check", "build", "run"] {
                        rejected(p.native(&[command], workers), diagnostic);
                    }
                }
            }
        }
    }
    let chain = Project::new(
        "e2021-let-chain",
        "edition = \"2021\"",
        "",
        "fn main() { let a = Some(1); if let Some(x) = a && x > 0 { println!(\"{}\", x); } }\n",
    );
    assert!(!chain.stock(&["check", "--offline"]).status.success());
    for workers in ["1", "8"] {
        rejected(
            chain.native(&["check"], workers),
            "let chains are only allowed in Rust 2024",
        );
    }
}

enum Pre2021 {
    /// Stock and native both run; stdout and exit status match.
    Run,
    /// Stock rejects; native rejects with this diagnostic.
    Rejects(&'static str),
    /// Stock accepts; native reports this explicit unsupported boundary.
    Boundary(&'static str),
}

#[test]
fn editions_2015_and_2018_match_stock_or_reject_explicitly() {
    use Pre2021::*;
    let both: &[&str] = &["2015", "2018", "omitted"];
    let cases: &[(&str, &[&str], &str, Pre2021)] = &[
        (
            "plain",
            both,
            "fn main() { let v = vec![1u32, 2, 3]; println!(\"{}\", v.iter().map(|x| x * 2).sum::<u32>()); }\n",
            Run,
        ),
        // The 2015/2018 prelude lacks FromIterator (and TryFrom/TryInto).
        (
            "prelude",
            both,
            "fn main() { let v: Vec<u8> = FromIterator::from_iter(vec![1u8, 2]); println!(\"{:?}\", v); }\n",
            Rejects("FromIterator"),
        ),
        // Whole-variable closure capture: borrowing `p.b` conflicts.
        (
            "capture-conflict",
            both,
            "struct P { a: String, b: String } fn main() { let mut p = P { a: \"x\".into(), b: \"y\".into() }; let mut c = || p.a.push_str(\"1\"); let r = &p.b; c(); println!(\"{} {}\", p.a, r); }\n",
            Rejects("conflicts"),
        ),
        // A move closure moves all of `p`, so `b` drops with the closure.
        (
            "capture-drop",
            both,
            "struct D(&'static str); impl Drop for D { fn drop(&mut self) { println!(\"drop {}\", self.0); } } struct P { a: D, b: D } fn main() { let p = P { a: D(\"a\"), b: D(\"b\") }; { let c = move || { let _x = &p.a; }; c(); println!(\"closure done\"); } println!(\"end\"); }\n",
            Run,
        ),
        // `:pat` excludes top-level or-patterns before 2021.
        (
            "pat-fragment",
            both,
            "macro_rules! m { ($p:pat) => { match 2u8 { $p => println!(\"hit\"), _ => println!(\"miss\") } } } fn main() { m!(1 | 2); }\n",
            Rejects("no macro rule matched"),
        ),
        // Single-literal panic-family messages are not format strings.
        (
            "panic-literal",
            both,
            "fn main() { panic!(\"a {{ b {}\"); }\n",
            Run,
        ),
        (
            "unreachable-literal",
            both,
            "fn main() { unreachable!(\"x {}\"); }\n",
            Run,
        ),
        (
            "assert-literal",
            both,
            "fn main() { assert!(1 == 2, \"x {}\"); }\n",
            Run,
        ),
        (
            "keywords-2015",
            &["2015", "omitted"],
            "fn main() { let async = 1; let try = 2; let dyn = 3; println!(\"{}\", async + try + dyn); }\n",
            Run,
        ),
        (
            "keywords-2018",
            &["2018"],
            "fn main() { let async = 1; println!(\"{}\", async); }\n",
            Rejects("async"),
        ),
        // 2015 `use` paths are crate-relative; 2018 requires `crate::`.
        (
            "use-2015",
            &["2015", "omitted"],
            "mod a { pub fn f() -> u8 { 1 } } mod b { use a::f; pub fn g() -> u8 { f() } } fn main() { println!(\"{}\", b::g()); }\n",
            Run,
        ),
        (
            "use-2018",
            &["2018"],
            "mod a { pub fn f() -> u8 { 1 } } mod b { use a::f; pub fn g() -> u8 { f() } } fn main() { println!(\"{}\", b::g()); }\n",
            Rejects("unresolved"),
        ),
        (
            "bare-trait",
            both,
            "trait T { fn f(&self) -> u8; } impl T for u8 { fn f(&self) -> u8 { *self } } fn g(x: &T) -> u8 { x.f() } fn main() { println!(\"{}\", g(&3u8)); }\n",
            Boundary("without `dyn`"),
        ),
        (
            "anonymous-param",
            &["2015"],
            "trait T { fn f(&self, u8) -> u8; } impl T for u8 { fn f(&self, x: u8) -> u8 { x } } fn main() { println!(\"{}\", 3u8.f(4)); }\n",
            Boundary("anonymous trait method parameters"),
        ),
        (
            "array-into-iter",
            both,
            "fn main() { for x in [1u32, 2].into_iter() { let y: &u32 = x; println!(\"{}\", y); } }\n",
            // Array by-value iteration is an unsupported constraint in every edition today;
            // the pre-2021 autoref boundary in type_check sits behind it.
            Boundary("unsupported"),
        ),
    ];
    for (name, editions, source, outcome) in cases {
        for edition in *editions {
            let field = if *edition == "omitted" {
                String::new()
            } else {
                format!("edition = \"{edition}\"")
            };
            let label = format!("{name}-{edition}");
            let p = Project::new(&format!("pre2021-{label}"), &field, "", source);
            let stock = p.stock(&["run", "--offline", "--quiet"]);
            for workers in ["1", "8"] {
                match outcome {
                    Run => {
                        let native = p.native(&["run"], workers);
                        assert_eq!(native.stdout, stock.stdout, "{label}");
                        if stock.status.success() {
                            assert!(native.status.success(), "{label}: {}", stderr(&native));
                            assert!(p.native(&["check"], workers).status.success(), "{label}");
                        } else {
                            // Panics (native aborts, stock unwinds): same payload line.
                            assert!(!native.status.success(), "{label}");
                            let message = |o: &Output| {
                                let text = stderr(o);
                                let at = text.find("panicked at").expect("panic header");
                                text[at..].lines().nth(1).unwrap_or_default().to_owned()
                            };
                            assert_eq!(message(&native), message(&stock), "{label}");
                        }
                    }
                    Rejects(diagnostic) => {
                        assert!(
                            !p.stock(&["check", "--offline"]).status.success(),
                            "{label}"
                        );
                        for command in ["check", "build"] {
                            rejected(p.native(&[command], workers), diagnostic);
                        }
                    }
                    Boundary(diagnostic) => {
                        assert!(stock.status.success(), "{label}: {}", stderr(&stock));
                        for command in ["check", "build", "run"] {
                            rejected(p.native(&[command], workers), diagnostic);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn unchecked_overflow_is_not_silently_compiled_as_checked() {
    let p = Project::new(
        "unchecked",
        "edition = \"2024\"",
        "[profile.release]\noverflow-checks = false",
        "fn increment(x: u8) -> u8 { x + 1 }\nfn main() { println!(\"{}\", increment(255)); }\n",
    );
    let stock = p.stock(&["run", "--release", "--offline", "--quiet"]);
    assert!(stock.status.success(), "{}", stderr(&stock));
    assert_eq!(stock.stdout, b"0\n");
    for workers in ["1", "8"] {
        let native = p.native(&["run", "--release"], workers);
        assert!(native.status.success(), "{}", stderr(&native));
        assert_eq!(native.stdout, stock.stdout);
        assert!(stderr(&native).contains("native unoptimized"));
        assert!(!stderr(&native).contains("[optimized]"));
    }
}

#[test]
fn direct_source_edition_is_explicit_and_never_reinterpreted() {
    let p = Project::new(
        "direct-edition",
        "edition = \"2024\"",
        "",
        "fn main() { println!(\"42\"); }\n",
    );
    let stock = p.stock(&["run", "--offline", "--quiet"]);
    assert!(stock.status.success(), "{}", stderr(&stock));
    for workers in ["1", "8"] {
        for flags in [vec![], vec!["--edition=2024"], vec!["--edition", "2024"]] {
            let binary = p.0.join(format!("direct-{workers}-{}", flags.len()));
            let checked = Command::new(env!("CARGO_BIN_EXE_jr-rust"))
                .arg(p.0.join("src/main.rs"))
                .args(&flags)
                .args(["--workers", workers, "--check"])
                .output()
                .unwrap();
            assert!(checked.status.success(), "{}", stderr(&checked));
            assert!(!binary.exists());
            let built = Command::new(env!("CARGO_BIN_EXE_jr-rust"))
                .arg(p.0.join("src/main.rs"))
                .args(&flags)
                .args(["--workers", workers, "-o"])
                .arg(&binary)
                .output()
                .unwrap();
            assert!(built.status.success(), "{}", stderr(&built));
            let ran = Command::new(&binary).output().unwrap();
            assert!(ran.status.success());
            assert_eq!(ran.stdout, stock.stdout);
        }
        for edition in ["2099", "2017", ""] {
            let binary = p.0.join(format!("rejected-{workers}-{edition}"));
            let output = Command::new(env!("CARGO_BIN_EXE_jr-rust"))
                .arg(p.0.join("absent.rs"))
                .arg(format!("--edition={edition}"))
                .args(["--workers", workers, "-o"])
                .arg(&binary)
                .output()
                .unwrap();
            assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
            assert!(stderr(&output).contains("edition"), "{}", stderr(&output));
            assert!(!stderr(&output).contains("No such file"));
            assert!(!binary.exists());
        }
    }
    for flags in [
        vec!["--edition"],
        vec!["--edition=2024", "--edition", "2024"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_jr-rust"))
            .arg(p.0.join("src/main.rs"))
            .arg("--check")
            .args(flags)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert!(stderr(&output).contains("--edition"));
    }
}

#[test]
fn supported_debug_assertions_match_stock_output() {
    for assertions in [true, false] {
        let p = Project::new(
            &format!("debug-{assertions}"),
            "edition = \"2024\"",
            &format!(
                "[profile.dev]\ndebug-assertions = {assertions}\noverflow-checks = true\nopt-level = 0"
            ),
            "#[cfg(debug_assertions)] fn assertions() -> bool { true }\n#[cfg(not(debug_assertions))] fn assertions() -> bool { false }\nfn main() { println!(\"{}\", assertions()); }\n",
        );
        let stock = p.stock(&["run", "--offline", "--quiet"]);
        assert!(stock.status.success(), "{}", stderr(&stock));
        for workers in ["1", "8"] {
            let native = p.native(&["run"], workers);
            assert!(native.status.success(), "{}", stderr(&native));
            assert_eq!(native.stdout, stock.stdout);
        }
    }
}

#[test]
fn default_and_custom_profile_runtime_policies_match_stock() {
    let source = "#[cfg(debug_assertions)] fn assertions() -> bool { true }\n#[cfg(not(debug_assertions))] fn assertions() -> bool { false }\nfn increment(x: u8) -> u8 { x + 1 }\nfn main() { println!(\"{}\", assertions()); println!(\"{}\", increment(255)); }\n";
    for (name, profile, args, wraps) in [
        ("dev", "", vec!["run"], false),
        ("release", "", vec!["run", "--release"], true),
        (
            "checked-release",
            "[profile.release]\noverflow-checks = true",
            vec!["run", "--release"],
            false,
        ),
        (
            "custom",
            "[profile.base]\ninherits = \"release\"\ndebug-assertions = true\n[profile.quick]\ninherits = \"base\"\nopt-level = 0",
            vec!["run", "--profile", "quick"],
            true,
        ),
    ] {
        let p = Project::new(name, "edition = \"2024\"", profile, source);
        let mut stock_args = args.clone();
        stock_args.extend(["--offline", "--quiet"]);
        let stock = p.stock(&stock_args);
        assert_eq!(stock.status.success(), wraps, "{}", stderr(&stock));
        for workers in ["1", "8"] {
            let native = p.native(&args, workers);
            assert_eq!(
                native.status.success(),
                stock.status.success(),
                "{}",
                stderr(&native)
            );
            assert_eq!(native.stdout, stock.stdout);
            if !wraps {
                assert!(stderr(&native).contains("overflow"), "{}", stderr(&native));
            }
        }
    }
}

#[test]
fn invalid_and_unsupported_profile_settings_fail_before_compilation() {
    for (name, profile, diagnostic) in [
        (
            "opt",
            "[profile.dev]\nopt-level = 4",
            "profile.dev.opt-level: invalid value",
        ),
        (
            "lto",
            "[profile.release]\nlto = \"wide\"",
            "profile.release.lto: invalid value",
        ),
        (
            "debug",
            "[profile.bench]\ndebug = 7",
            "profile.bench.debug: invalid value",
        ),
        (
            "panic",
            "[profile.dev]\npanic = \"unwind\"",
            "profile.dev.panic: profile setting is unsupported",
        ),
        (
            "override",
            "[profile.dev.package.other]\noverflow-checks = false",
            "profile.dev.package: profile setting is unsupported",
        ),
        (
            "missing-parent",
            "[profile.quick]\ndebug-assertions = false",
            "custom profiles require an inherits setting",
        ),
        (
            "unknown-parent",
            "[profile.quick]\ninherits = \"missing\"",
            "profile `missing` is not defined",
        ),
        (
            "cycle",
            "[profile.a]\ninherits = \"b\"\n[profile.b]\ninherits = \"a\"",
            "cyclic profile inheritance",
        ),
        (
            "bad-bool",
            "[profile.dev]\noverflow-checks = \"false\"",
            "expected a boolean",
        ),
        (
            "builtin-parent",
            "[profile.dev]\ninherits = \"release\"",
            "built-in profiles cannot specify inheritance",
        ),
    ] {
        let p = Project::new(name, "edition = \"2024\"", profile, "fn main() {}\n");
        for workers in ["1", "8"] {
            for command in ["check", "build", "run"] {
                rejected(p.native(&[command], workers), diagnostic);
            }
        }
    }
    let p = Project::new("undefined", "edition = \"2024\"", "", "fn main() {}\n");
    assert!(
        !p.stock(&["check", "--profile", "missing", "--offline"])
            .status
            .success()
    );
    rejected(
        p.native(&["check", "--profile", "missing"], "1"),
        "profile `missing` is not defined",
    );
}

#[test]
fn workspace_inherited_edition_is_validated() {
    for edition in ["2024", "2099"] {
        let p = Project::new(
            &format!("inherited-{edition}"),
            "edition.workspace = true",
            "",
            "fn main() {}\n",
        );
        let manifest = std::fs::read_to_string(p.0.join("Cargo.toml")).unwrap();
        std::fs::write(
            p.0.join("Cargo.toml"),
            format!("{manifest}\n[workspace]\n[workspace.package]\nedition = \"{edition}\"\n"),
        )
        .unwrap();
        let stock = p.stock(&["check", "--offline"]);
        assert_eq!(
            stock.status.success(),
            edition == "2024",
            "{}",
            stderr(&stock)
        );
        for workers in ["1", "8"] {
            let native = p.native(&["check"], workers);
            assert_eq!(
                native.status.success(),
                stock.status.success(),
                "{}",
                stderr(&native)
            );
            if edition == "2099" {
                rejected(native, "invalid Rust edition `2099`");
            }
        }
    }
}

#[test]
fn test_profile_overrides_dev_and_conflicting_selection_is_rejected() {
    let p = Project::new(
        "test-profile",
        "edition = \"2024\"",
        "[profile.dev]\ndebug-assertions = false\n[profile.test]\ndebug-assertions = true",
        "#[cfg(debug_assertions)] fn assertions() -> bool { true }\n#[cfg(not(debug_assertions))] fn assertions() -> bool { false }\nfn main() {}\n#[test] fn profile_assertions() { assert!(assertions()); }\n",
    );
    let stock = p.stock(&["test", "--offline", "--quiet"]);
    assert!(stock.status.success(), "{}", stderr(&stock));
    for workers in ["1", "8"] {
        let native = p.native(&["test"], workers);
        assert!(native.status.success(), "{}", stderr(&native));
        assert!(String::from_utf8_lossy(&native.stdout).contains("1 passed"));
        rejected(
            p.native(&["check", "--release", "--profile", "dev"], workers),
            "cannot be used together",
        );
    }
    assert!(
        !p.stock(&["check", "--release", "--profile", "dev", "--offline"])
            .status
            .success()
    );
}
