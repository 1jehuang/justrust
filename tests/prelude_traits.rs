//! Source trait contracts, not compiler-recognized method names or desugaring.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

use justrust_engine::compiler;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn fixture(source: &str, unsupported: Option<&str>) {
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
    assert!(
        output.status.success(),
        "stock rejected source: {}\n{source}",
        String::from_utf8_lossy(&output.stderr)
    );
    let expected = Command::new(&stock).output().unwrap();
    assert!(expected.status.success());
    for workers in [1, 8] {
        for check in [true, false] {
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
            if let Some(executable) = compiled.executable {
                fs::write(&native, executable).unwrap();
                fs::set_permissions(&native, fs::Permissions::from_mode(0o700)).unwrap();
                let actual = Command::new(&native).output().unwrap();
                assert_eq!(actual.status.code(), expected.status.code());
                assert_eq!(actual.stdout, expected.stdout);
                assert_eq!(actual.stderr, expected.stderr);
            }
        }
    }
    fs::remove_dir_all(dir).unwrap();
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
