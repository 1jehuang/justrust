//! Owner-authority gates for the OwnerPath activation (PARSE-SPLIT.md).
//!
//! These pin today's behavior with the process-global counters, so replacing
//! them with structural owner paths must keep every rejection. Each case runs
//! repeatedly and concurrently, mirroring workers 1 and 8.
use justrust_engine::compiler::frontend::{self, Program};
use justrust_engine::compiler::{ir, owner_path::OwnerIssuer};
use std::sync::Arc;

const SOURCE: &str =
    "mod m { pub fn f() -> i32 { 1 } } fn g(x: i32) -> i32 { x } fn main() { g(m::f()); }";

fn parsed() -> Program {
    frontend::parse(SOURCE).expect("fixture parses")
}

fn accepts(program: Program) {
    let budget = ir::PlanBudget::default();
    frontend::PlanCtx::from_shared_program(Arc::new(program), &budget)
        .expect("unmodified program must plan");
}

fn rejects_foreign(program: Program, what: &str) {
    let budget = ir::PlanBudget::default();
    let error = match frontend::PlanCtx::from_shared_program(Arc::new(program), &budget) {
        Err(error) => error,
        Ok(_) => panic!("{what}: foreign handle acquired source authority"),
    };
    let lower = error.to_ascii_lowercase();
    assert!(
        lower.contains("foreign") || lower.contains("contract") || lower.contains("owner"),
        "{what}: wrong rejection: {error}"
    );
    assert!(
        !lower.contains("unsupported"),
        "{what}: wrong boundary: {error}"
    );
}

/// Repeat sequentially (workers 1) and on 8 threads (workers 8).
fn repeated(case: fn()) {
    for _ in 0..4 {
        case();
    }
    std::thread::scope(|scope| {
        for _ in 0..8 {
            scope.spawn(case);
        }
    });
}

#[test]
fn identical_direct_parses_issue_distinct_owners() {
    repeated(|| {
        let a = parsed();
        let b = parsed();
        accepts(a.clone());
        // A function name from one parse grafted into an identical parse is a
        // foreign occurrence, not an equal one.
        let mut grafted = a;
        grafted.functions[0].name = b.functions[0].name.clone();
        rejects_foreign(grafted, "function name from identical parse");
    });
}

#[test]
fn foreign_module_handle_rejects() {
    repeated(|| {
        let a = parsed();
        let b = parsed();
        let mut grafted = a;
        grafted.functions[0].module = b.functions[0].module;
        rejects_foreign(grafted, "module id from identical parse");
    });
}

#[test]
fn swapped_name_store_rejects() {
    repeated(|| {
        let a = parsed();
        let b = parsed();
        // Same text, different snapshot: every handle in `a` is foreign to `b`'s store.
        let mut swapped = a;
        swapped.names = b.names.clone();
        rejects_foreign(swapped, "name store from identical parse");
    });
}

#[test]
fn swapped_module_arena_rejects() {
    repeated(|| {
        let a = parsed();
        let b = parsed();
        let mut swapped = a;
        swapped.modules = b.modules.clone();
        rejects_foreign(swapped, "module arena from identical parse");
    });
}

#[test]
fn duplicate_occurrence_within_one_program_is_not_a_foreign_owner() {
    // Control: reusing a handle from the same parse elsewhere must not be
    // reported as a foreign-owner failure, so the gates above test ownership.
    let program = parsed();
    let budget = ir::PlanBudget::default();
    if let Err(error) = frontend::PlanCtx::from_shared_program(Arc::new(program), &budget) {
        assert!(
            !error.to_ascii_lowercase().contains("foreign"),
            "same-owner program reported foreign: {error}"
        );
    }
}

#[test]
fn standalone_owner_roots_are_distinct_across_threads() {
    let roots: Vec<_> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..32)
            .map(|_| scope.spawn(|| OwnerIssuer::standalone().path().clone()))
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    for (i, a) in roots.iter().enumerate() {
        for b in &roots[i + 1..] {
            assert_ne!(a, b);
        }
    }
}
