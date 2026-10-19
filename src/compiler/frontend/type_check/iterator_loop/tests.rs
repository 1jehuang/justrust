//! Engine-level fixture supplies canonical library declarations, not the full
//! library. The unchanged user program is also built and run by stock rustc.
use super::*;

const USER: &str = r#"
use std::option::Option::{Some, None};
use std::iter::Iterator;
struct Counter { n: i32 }
impl Iterator for Counter {
    type Item = i32;
    fn next(&mut self) -> std::option::Option<i32> {
        if self.n < 4 { self.n += 1; Some(self.n) } else { None }
    }
}
fn make() -> Counter { println!("make"); Counter { n: 0 } }
fn main() {
    let mut total = 0;
    'outer: for x in make() {
        if x == 2 { continue 'outer; }
        total += x;
        if x == 3 { break 'outer; }
    }
    println!("{}", total);
}
"#;

const LIBRARY: &str = r#"
mod std {
    pub mod option { pub enum Option<T> { None, Some(T) } }
    pub mod iter {
        use super::option::Option;
        pub trait Iterator {
            type Item;
            fn next(&mut self) -> Option<Self::Item>;
        }
        pub trait IntoIterator {
            type Item;
            type IntoIter;
            fn into_iter(self) -> Self::IntoIter;
        }
        impl<I: Iterator> IntoIterator for I {
            type Item = <I as Iterator>::Item;
            type IntoIter = I;
            fn into_iter(self) -> I { self }
        }
    }
}
"#;

#[test]
fn blanket_into_iterator_for_calls_match_stock() {
    stock_matching_loop(LIBRARY, USER, "blanket", b"make\n4\n");
}

const EXPLICIT_LIBRARY: &str = r#"
mod std {
    pub mod option { pub enum Option<T> { None, Some(T) } }
    pub mod iter {
        use super::option::Option;
        pub trait Iterator { type Item; fn next(&mut self) -> Option<Self::Item>; }
        pub trait IntoIterator {
            type Item; type IntoIter; fn into_iter(self) -> Self::IntoIter;
        }
    }
}
impl std::iter::IntoIterator for Counter {
    type Item = i32;
    type IntoIter = Counter;
    fn into_iter(self) -> Counter { self }
}
"#;

fn fixture(library: &str, user: &str) -> Program {
    let mut program = parse(&format!("{library}{user}")).unwrap();
    program.prelude_module = program.modules.parent(program.traits[0].module).unwrap();
    program
}

#[test]
fn canonical_for_calls_check_build_run_at_workers_one_and_eight() {
    stock_matching_loop(EXPLICIT_LIBRARY, USER, "explicit", b"make\n4\n");
}

#[test]
fn canonical_for_tuple_items_match_stock() {
    let user = USER
        .replace("type Item = i32;", "type Item = (i32, i32);")
        .replace("Option<i32>", "Option<(i32, i32)>")
        .replace("Some(self.n)", "Some((self.n, self.n * 10))")
        .replace("for x in make()", "for (x, y) in make()")
        .replace("total += x;", "total += x + y;");
    let library = EXPLICIT_LIBRARY.replace("type Item = i32;", "type Item = (i32, i32);");
    stock_matching_loop(&library, &user, "tuple", b"make\n44\n");
}

#[test]
fn canonical_for_receiver_arguments_match_stock() {
    let user = format!(
        "{}{}",
        USER.replace("for x in make()", "for x in make().identity(7)"),
        "impl Counter { fn identity(self, n: i32) -> Counter { println!(\"{}\", n); self } }"
    );
    stock_matching_loop(EXPLICIT_LIBRARY, &user, "receiver", b"make\n7\n4\n");
}

#[test]
fn canonical_for_adapter_name_is_not_peeled() {
    let user = format!(
        "{}{}",
        USER.replace("for x in make()", "for x in make().rev()"),
        "impl Counter { fn rev(self) -> Counter { println!(\"rev\"); self } }"
    );
    stock_matching_loop(EXPLICIT_LIBRARY, &user, "written-rev", b"make\nrev\n4\n");
}

#[test]
fn canonical_for_unicode_char_items_match_stock() {
    let user = r#"
use std::iter::Iterator;
use std::option::Option::{Some, None};
struct Counter { n: i32 }
impl Iterator for Counter {
    type Item = char;
    fn next(&mut self) -> std::option::Option<char> {
        if self.n == 0 { self.n = 1; Some('水') }
        else if self.n == 1 { self.n = 2; Some('🦀') }
        else { None }
    }
}
fn make() -> Counter { println!("make"); Counter { n: 0 } }
fn main() {
    let mut total = 0;
    for c in make() { if c == '水' { total += 1; } else { total += 2; } }
    println!("{}", total);
}
"#;
    let library = EXPLICIT_LIBRARY.replace("type Item = i32;", "type Item = char;");
    stock_matching_loop(&library, user, "char", b"make\n3\n");
}

#[test]
fn canonical_for_rejects_refutable_pattern() {
    let prefix = USER.split("fn main()").next().unwrap();
    let user = format!("{prefix}fn main() {{ for 0 in make() {{ println!(\"bad\"); }} }}");
    let root = std::env::var_os("JCODE_SCRATCH_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let source = root.join(format!("jr-iterator-refutable-{}.rs", std::process::id()));
    std::fs::write(&source, &user).unwrap();
    let compiler = std::env::var_os("JUSTRUST_REAL_RUSTC").unwrap_or_else(|| "rustc".into());
    let output = std::process::Command::new(compiler)
        .args([
            "--edition=2024",
            "--emit=metadata",
            "--crate-name=iterator_refutable",
        ])
        .arg(&source)
        .arg("-o")
        .arg(source.with_extension("rmeta"))
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "stock accepted a refutable for pattern"
    );
    for workers in [1, 8] {
        for check in [true, false] {
            assert!(
                crate::compiler::compile_program_observed(
                    fixture(EXPLICIT_LIBRARY, &user),
                    workers,
                    check
                )
                .is_err(),
                "native accepted refutable for pattern workers {workers}, check {check}"
            );
        }
    }
    let _ = std::fs::remove_file(&source);
    let _ = std::fs::remove_file(source.with_extension("rmeta"));
}

#[test]
fn canonical_for_rejects_missing_implicit_operations() {
    for program in [
        fixture(EXPLICIT_LIBRARY, USER),
        parse("fn main(){for x in [1,2,3]{println!(\"{}\",x);}}").unwrap(),
    ] {
        let budget = ir::PlanBudget::default();
        let plan = PlanCtx::from_program(&program, &budget).unwrap();
        let declaration = plan.function_decl(program.functions.len() - 1).unwrap();
        let PublishedCallableBody::Source(publication) =
            plan.publish_callable_body(declaration, &budget).unwrap()
        else {
            panic!("expected canonical source publication");
        };
        let original = publication.original().source_for_test();
        let length = original
            .calls
            .iter()
            .position(|site| site.for_iterable)
            .unwrap();
        let first = length - 2;
        let mut missing = original.clone();
        missing
            .call_mapping
            .retain(|(index, _, _)| *index != first && *index != first + 1);
        missing.builtin_len_mapping.clear();
        let error =
            replay_source_body(&missing, publication.source(), publication.facts(), &budget)
                .unwrap_err();
        assert_eq!(
            error.kind(),
            crate::compiler::traits::FactsErrorKind::InvalidMetadata
        );
        assert!(
            error
                .into_legacy_diagnostic(&budget)
                .contains("for occurrence has incomplete or mixed desugaring")
        );
    }
}

fn stock_matching_loop(library: &str, user: &str, case: &str, stdout: &[u8]) {
    use std::os::unix::fs::PermissionsExt;
    let root = std::env::var_os("JCODE_SCRATCH_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let stem = root.join(format!("jr-iterator-loop-{}-{case}", std::process::id()));
    let source = stem.with_extension("rs");
    let stock_image = stem.with_extension("stock");
    std::fs::write(&source, user).unwrap();
    let compiler = std::env::var_os("JUSTRUST_REAL_RUSTC").unwrap_or_else(|| "rustc".into());
    let stock = std::process::Command::new(compiler)
        .args([
            "--edition=2024",
            "-Cpanic=abort",
            "--crate-name=iterator_loop_oracle",
        ])
        .arg(&source)
        .arg("-o")
        .arg(&stock_image)
        .output()
        .unwrap();
    assert!(
        stock.status.success(),
        "{}",
        String::from_utf8_lossy(&stock.stderr)
    );
    let expected = std::process::Command::new(&stock_image).output().unwrap();
    assert!(expected.status.success(), "{expected:?}");
    assert_eq!(expected.stdout, stdout);
    for workers in [1, 8] {
        let checked =
            crate::compiler::compile_program_observed(fixture(library, user), workers, true)
                .unwrap_or_else(|error| panic!("CHECK workers {workers}: {error}"));
        assert!(checked.executable.is_none());
        assert!(checked.report.capture_complete);
        assert!(
            checked
                .report
                .node_evaluations
                .iter()
                .all(|(key, _)| { !matches!(key.kind, 103 | 105 | 130..=133) }),
            "CHECK demanded native code generation"
        );
        let built =
            crate::compiler::compile_program_observed(fixture(library, user), workers, false)
                .unwrap_or_else(|error| panic!("BUILD workers {workers}: {error}"));
        let executable = built.executable.expect("BUILD must publish an executable");
        let path = stem.with_extension(format!("native-{workers}"));
        std::fs::write(&path, executable).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        let output = std::process::Command::new(&path).output().unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(output.status.code(), expected.status.code(), "{output:?}");
        assert_eq!(output.stdout, expected.stdout);
        assert_eq!(output.stderr, expected.stderr);
    }
    let _ = std::fs::remove_file(source);
    let _ = std::fs::remove_file(stock_image);
}
