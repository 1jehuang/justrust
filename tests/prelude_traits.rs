//! Source trait contracts, not compiler-recognized method names or desugaring.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

#[path = "support/native_prelude.rs"]
mod native_prelude;

use justrust_engine::Key;
use justrust_engine::compiler;
use justrust_engine::compiler::frontend::{CallableProvider, PlanCtx};
use justrust_engine::compiler::ir::PlanBudget;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn fixture(source: &str, unsupported: Option<&str>) {
    fixture_case(source, unsupported, None);
}

fn fixture_case(source: &str, unsupported: Option<&str>, stock_invalid: Option<&str>) {
    let base = std::env::var_os("JCODE_SCRATCH_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let dir = base.join(format!(
        "jr-prelude-traits-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&dir).unwrap();
    let input = dir.join("input.rs");
    let stock = dir.join("stock");
    let native = dir.join("native");
    fs::write(&input, source).unwrap();
    let output = Command::new("rustc")
        .args([
            "--edition=2024",
            "-Cpanic=abort",
            "--crate-name=prelude_traits",
        ])
        .arg(&input)
        .arg("-o")
        .arg(&stock)
        .output()
        .expect("stock rustc source-trait oracle must be installed");
    let stock_stderr = String::from_utf8_lossy(&output.stderr);
    let expected = if let Some(diagnostic) = stock_invalid {
        assert!(!output.status.success(), "stock admitted invalid source");
        assert!(stock_stderr.contains(diagnostic), "{stock_stderr}");
        None
    } else {
        assert!(
            output.status.success(),
            "stock rejected source: {stock_stderr}\n{source}"
        );
        let expected = Command::new(&stock).output().unwrap();
        assert!(expected.status.success());
        Some(expected)
    };
    for workers in [1, 8] {
        for check in [true, false] {
            if stock_invalid.is_some() {
                let failure = compiler::compile_observed(source.into(), workers, check)
                    .err()
                    .expect("native admitted stock-invalid source");
                let diagnostic = failure.error.to_string();
                assert!(!diagnostic.contains("unsupported:"), "{diagnostic}");
                let report = failure.report.unwrap();
                assert!(report.capture_complete);
                assert!(report.failures > 0);
                assert!(!report.published_kind(105));
                continue;
            }
            let result = compiler::compile(source.into(), workers, check);
            if let Some(boundary) = unsupported {
                let error = result.err().expect("unsupported source contract admitted");
                let diagnostic = error.to_string();
                assert!(diagnostic.contains("unsupported:"), "{diagnostic}");
                assert!(diagnostic.contains(boundary), "{diagnostic}");
                continue;
            }
            let compiled = result.unwrap_or_else(|error| panic!("{error}\n{source}"));
            assert!(compiled.analyses.iter().all(|analysis| analysis.passed()));
            assert_eq!(compiled.executable.is_none(), check);
            assert_graph_authority(source, &compiled, check);
            if let Some(executable) = compiled.executable {
                fs::write(&native, executable).unwrap();
                fs::set_permissions(&native, fs::Permissions::from_mode(0o700)).unwrap();
                let actual = Command::new(&native).output().unwrap();
                let expected = expected.as_ref().unwrap();
                assert_eq!(actual.status.code(), expected.status.code());
                assert_eq!(actual.stdout, expected.stdout);
                assert_eq!(actual.stderr, expected.stderr);
            }
        }
    }
    fs::remove_dir_all(dir).unwrap();
}

fn assert_graph_authority(source: &str, compiled: &compiler::Compilation, check: bool) {
    let program = native_prelude::parse(source).unwrap();
    let plan = PlanCtx::from_program(&program, &PlanBudget::default()).unwrap();
    let report = &compiled.report;
    assert!(report.capture_complete);
    assert_eq!(report.events_dropped, 0);
    assert_eq!(report.failures, 0);
    assert_eq!(compiled.source_bytes, source.len());
    assert_eq!(compiled.source_files, 1);
    let evaluated_once = |key: &Key| {
        let evaluations: Vec<_> = report
            .node_evaluations
            .iter()
            .filter(|(seen, _)| seen == key)
            .collect();
        assert_eq!(evaluations.len(), 1, "missing or duplicated {key:?}");
        assert_eq!(evaluations[0].1, 1, "repeated {key:?}");
    };
    for index in 0..plan.callable_count() {
        let declaration = plan.callable_decl(index).unwrap();
        evaluated_once(&Key::new(110, vec![index as u64]));
        let universal = match plan.callable_provider(declaration).unwrap() {
            CallableProvider::Source { .. } => plan.callable_env(declaration).unwrap().is_generic(),
            CallableProvider::Recipe { .. } => true,
            CallableProvider::UnavailableProvided { .. } => panic!("unavailable callable"),
        };
        if universal {
            for kind in [111, 112, 115] {
                evaluated_once(&Key::new(kind, vec![index as u64]));
            }
        }
    }
    for key in native_prelude::library_source_keys() {
        evaluated_once(&key);
    }
    let published = |key: &Key| {
        let spelling = format!("{key:?}");
        let node = report
            .events
            .iter()
            .find(|event| event.kind == "discovered" && event.detail == spelling)
            .unwrap()
            .node
            .unwrap();
        report
            .events
            .iter()
            .find(|event| event.kind == "publish" && event.node == Some(node))
            .unwrap()
            .at_ns
    };
    for (key, _) in &report.node_evaluations {
        if key.kind == 103 {
            assert!(!check, "check must not emit objects");
            let proof = Key::new(109, key.params.clone());
            assert!(published(&proof) <= published(key));
        }
    }
}

#[test]
fn exact_source_iterator_requires_nominal_projection_support() {
    fixture(
        r#"
mod contracts {
    pub trait Iterator {
        type Item;
        fn next(&mut self) -> Option<Self::Item>;
    }
}
use contracts::Iterator;
struct Count { remaining: i32 }
impl Iterator for Count {
    type Item = i32;
    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining > 0 {
            self.remaining = self.remaining - 1;
            Some(self.remaining)
        } else { None }
    }
}
fn main() {
    let mut iterator = Count { remaining: 2 };
    loop {
        match iterator.next() {
            Some(value) => println!("{}", value),
            None => break,
        }
    }
    if <Count as Iterator>::next(&mut iterator).is_none() { println!("done"); }
}
"#,
        Some("residual associated type projection"),
    );
}

#[test]
fn source_projection_nested_enum_inside_struct_requires_authoritative_remint() {
    fixture(
        r#"
struct Envelope<T> { payload: T }
enum Choice<T> { Value(T), Empty }
trait Read { type Item; fn read(&self) -> Envelope<Choice<Self::Item>>; }
struct Source { value: i32 }
impl Read for Source {
    type Item = i32;
    fn read(&self) -> Envelope<Choice<Self::Item>> {
        Envelope { payload: Choice::Value(self.value) }
    }
}
fn main() {
    let source = Source { value: 17 };
    match source.read().payload {
        Choice::Value(value) => println!("{}", value),
        Choice::Empty => println!("empty"),
    }
}
"#,
        Some("residual associated type projection"),
    );
}

#[test]
fn source_projection_nested_struct_inside_enum_requires_authoritative_remint() {
    fixture(
        r#"
struct Payload<T> { value: T }
enum Reply<T> { Present(T), Absent }
trait Take { type Item; fn take(&mut self) -> Reply<Payload<Self::Item>>; }
struct Source { value: i32 }
impl Take for Source {
    type Item = i32;
    fn take(&mut self) -> Reply<Payload<Self::Item>> {
        let value = self.value;
        self.value = self.value + 1;
        Reply::Present(Payload { value })
    }
}
fn main() {
    let mut source = Source { value: 23 };
    match <Source as Take>::take(&mut source) {
        Reply::Present(payload) => println!("{}", payload.value),
        Reply::Absent => println!("absent"),
    }
    match source.take() {
        Reply::Present(payload) => println!("{}", payload.value),
        Reply::Absent => println!("absent"),
    }
}
"#,
        Some("residual associated type projection"),
    );
}

#[test]
fn exact_deref_relaxed_target_is_an_explicit_boundary() {
    fixture(
        "mod contracts { pub trait Deref { type Target: ?Sized; fn deref(&self) -> &Self::Target; } } fn main() {}",
        Some("unsupported token \"?\""),
    );
}

#[test]
fn exact_into_iterator_bound_is_an_explicit_boundary() {
    fixture(
        "mod contracts { pub trait Iterator { type Item; fn next(&mut self) -> Option<Self::Item>; } pub trait IntoIterator { type Item; type IntoIter: Iterator<Item = Self::Item>; fn into_iter(self) -> Self::IntoIter; } } fn main() {}",
        Some("associated"),
    );
}

#[test]
fn projected_value_results_allow_shared_and_mutable_receivers() {
    fixture(
        r#"
trait Read { type Item; fn read(&self) -> Self::Item; }
trait Take { type Item; fn take(&mut self) -> Self::Item; }
struct Counter { value: i32 }
impl Read for Counter {
    type Item = i32;
    fn read(&self) -> Self::Item { self.value }
}
impl Take for Counter {
    type Item = i32;
    fn take(&mut self) -> Self::Item {
        let old = self.value;
        self.value = self.value + 1;
        old
    }
}
fn main() {
    let mut counter = Counter { value: 5 };
    println!("{}", <Counter as Read>::read(&counter));
    {
        let carrier = &mut counter;
        println!("{}", carrier.take());
        println!("{}", <Counter as Take>::take(carrier));
    }
    println!("{}", counter.read());
}
"#,
        None,
    );
}

#[test]
fn projected_reference_results_are_not_admitted_by_receiver_support() {
    fixture(
        "trait Read { type Item; fn read(&self) -> &Self::Item; } struct S { value: i32 } impl Read for S { type Item = i32; fn read(&self) -> &Self::Item { &self.value } } fn main() {}",
        Some("reference-bearing projected method signature"),
    );
}

#[test]
fn projected_receiver_calls_preserve_mutable_alias_and_returned_loan_rejection() {
    const SOURCE: &str = "trait Take { type Item; fn take(&mut self) -> Self::Item; } struct Counter { value: i32 } impl Take for Counter { type Item = i32; fn take(&mut self) -> Self::Item { self.value } } ";
    fixture_case(
        &format!(
            "{SOURCE} fn main() {{ let mut counter = Counter {{ value: 1 }}; let first = &mut counter; let second = &mut counter; println!(\"{{}}\", first.take()); println!(\"{{}}\", second.take()); }}"
        ),
        None,
        Some("error[E0499]"),
    );
    fixture_case(
        &format!(
            "{SOURCE} fn escape<'a>(input: &'a mut Counter) -> &'a mut Counter {{ let mut counter = Counter {{ value: 1 }}; counter.take(); &mut counter }} fn main() {{}}"
        ),
        None,
        Some("error[E0515]"),
    );
}
