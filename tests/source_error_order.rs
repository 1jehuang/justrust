//! Pins which diagnostic wins when several source files are invalid.
//!
//! The per-file PARSE split (PARSE-SPLIT.md) must report exactly the error the
//! assembled-source pipeline reports today, at workers 1 and 8. These fixtures
//! cover parent errors before and after a `mod child;` declaration, child
//! errors, sibling order, and preparation versus parser precedence across files.
//! Today every file is prepared (delimiters, child loading) before parsing, so
//! any preparation error beats every parser error; parser errors then follow
//! assembled order, with each child spliced at its `mod child;` site.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

use justrust_engine::compiler;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Tree(PathBuf);
impl Tree {
    fn new(files: &[(&str, &str)]) -> Self {
        let base = std::env::var_os("JCODE_SCRATCH_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let root = base.join(format!(
            "jr-source-error-order-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&root);
        for (name, text) in files {
            let path = root.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
        Self(root)
    }
}
impl Drop for Tree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// The first diagnostic, identical for every worker count and mode.
fn first_error(files: &[(&str, &str)]) -> String {
    let tree = Tree::new(files);
    let mut seen: Option<String> = None;
    for workers in [1, 8] {
        for check in [true, false] {
            let error = match compiler::compile_file_observed(
                tree.0.join("src/input.rs"),
                workers,
                check,
            ) {
                Ok(_) => panic!("accepted invalid tree {files:?}"),
                Err(failure) => failure.error.to_string(),
            };
            if let Some(previous) = &seen {
                assert_eq!(
                    previous, &error,
                    "first error depends on workers={workers} check={check}"
                );
            }
            seen = Some(error);
        }
    }
    seen.unwrap()
}

fn expect(files: &[(&str, &str)], needles: &[&str], absent: &[&str]) {
    let error = first_error(files);
    for needle in needles {
        assert!(error.contains(needle), "expected {needle:?} in: {error}");
    }
    for needle in absent {
        assert!(!error.contains(needle), "unexpected {needle:?} in: {error}");
    }
}

/// A balanced-delimiter syntax error: reported by the parser, not by
/// per-file preparation.
const PARSER_ERROR: &str = "let = ;";

fn body(name: &str) -> String {
    format!("pub fn {name}() {{ {PARSER_ERROR} }}")
}

// ---- Parser errors (balanced delimiters): assembled source order. ----

#[test]
fn parser_error_parent_before_child_declaration_wins() {
    expect(
        &[
            (
                "src/input.rs",
                &format!("{}\nmod child;\nfn main() {{}}", body("early")),
            ),
            ("src/child.rs", &body("broken")),
        ],
        &["input.rs"],
        &["child.rs"],
    );
}

#[test]
fn parser_error_child_precedes_parent_text_after_declaration() {
    expect(
        &[
            (
                "src/input.rs",
                &format!("mod child;\n{}\nfn main() {{}}", body("late")),
            ),
            ("src/child.rs", &body("broken")),
        ],
        &["child.rs:1:23"],
        &["input.rs"],
    );
}

#[test]
fn parser_error_sibling_order_is_declaration_order() {
    expect(
        &[
            ("src/input.rs", "mod zeta;\nmod alpha;\nfn main() {}"),
            ("src/zeta.rs", &body("z")),
            ("src/alpha.rs", &body("a")),
        ],
        &["zeta.rs:1:18"],
        &["alpha.rs"],
    );
}

#[test]
fn parser_error_grandchild_versus_later_text() {
    expect(
        &[
            (
                "src/input.rs",
                &format!("mod outer;\n{}\nfn main() {{}}", body("late")),
            ),
            (
                "src/outer.rs",
                &format!("pub mod inner;\n{}", body("after")),
            ),
            ("src/outer/inner.rs", &body("deep")),
        ],
        &["outer/inner.rs:1:21"],
        &["input.rs", "outer.rs:"],
    );
}

// ---- Preparation errors (delimiters, missing files): per-file precedence. ----

#[test]
fn preparation_error_in_parent_after_child_declaration_wins_over_child() {
    // Delimiter balance is a per-file preparation fact. The parent's own
    // preparation error is reported before any child is spliced.
    expect(
        &[
            ("src/input.rs", "mod child;\nfn late( {}\nfn main() {}"),
            ("src/child.rs", "pub fn broken( {}"),
        ],
        &["input.rs:2:8", "unclosed delimiter"],
        &["child.rs"],
    );
}

#[test]
fn preparation_error_in_parent_wins_over_missing_child() {
    expect(
        &[("src/input.rs", "mod absent;\nfn late( {}\nfn main() {}")],
        &["input.rs:2:8", "unclosed delimiter"],
        &[],
    );
}

#[test]
fn missing_child_wins_over_later_parent_parser_error() {
    expect(
        &[(
            "src/input.rs",
            &format!("mod absent;\n{}\nfn main() {{}}", body("late")),
        )],
        &["absent"],
        &["expected ASCII identifier"],
    );
}

#[test]
fn child_preparation_error_versus_parent_parser_error() {
    // Every file is prepared before any parsing, so a child's preparation
    // error wins even over a parser error earlier in the parent.
    for parent in [
        format!("{}\nmod child;\nfn main() {{}}", body("early")),
        format!("mod child;\n{}\nfn main() {{}}", body("late")),
    ] {
        expect(
            &[
                ("src/input.rs", &parent),
                ("src/child.rs", "pub fn broken( {}"),
            ],
            &["child.rs:1:14", "unclosed delimiter"],
            &["input.rs"],
        );
    }
}
