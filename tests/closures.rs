//! First real closure callable increment. Stored environments remain open.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/oracle.rs"]
mod oracle;
use justrust_engine::compiler;
use oracle::{Case, Expect};

#[test]
fn immediate_noncapturing_closures_match_stock() {
    assert!(
        std::process::Command::new("rustc")
            .arg("-vV")
            .output()
            .is_ok_and(|o| o.status.success()),
        "local stock oracle required"
    );
    oracle::check(
        "closures",
        &[
            Case {
                name: "scalar",
                source: "fn main() { let a = (|x: i32| -> i32 { x + 1 })(41); assert_eq!(a, 42); println!(\"closure\"); }",
                expect: Expect::Run("closure\n"),
            },
            Case {
                name: "distinct_same_signature",
                source: "fn main() { assert_eq!((|x: i32| -> i32 { x + 1 })(3), 4); assert_eq!((|x: i32| -> i32 { x * 3 })(3), 9); }",
                expect: Expect::Run(""),
            },
            Case {
                name: "zero_args_move_return",
                source: "fn main() { assert_eq!((move || -> i32 { return 42; })(), 42); (|| -> () {})(); }",
                expect: Expect::Run(""),
            },
            Case {
                name: "qualified_function_and_module",
                source: "fn twice(x: i32) -> i32 { x * 2 } mod m { pub fn run() -> i32 { (|x: i32| -> i32 { crate::twice(x) })(21) } } fn main() { assert_eq!(m::run(), 42); }",
                expect: Expect::Run(""),
            },
            Case {
                name: "nested_distinct_locals",
                source: "fn main() { assert_eq!((|x: i32| -> i32 { (|x: i32| -> i32 { x * 2 })(x) + x })(7), 21); }",
                expect: Expect::Run(""),
            },
            Case {
                name: "argument_order_once",
                source: "fn mark(x: i32) -> i32 { println!(\"{}\", x); x } fn main() { let n = (|a: i32, b: i32| -> i32 { a * 10 + b })(mark(1), mark(2)); println!(\"{}\", n); }",
                expect: Expect::Run("1\n2\n12\n"),
            },
            Case {
                name: "parameter_borrow",
                source: "fn main() { let mut n = 1i32; let a = (|x: &mut i32| -> i32 { *x += 2; *x })(&mut n); assert_eq!(a, 3); assert_eq!(n, 3); }",
                expect: Expect::Run(""),
            },
            Case {
                name: "move_argument",
                source: "struct Value { n: i32 } fn main() { let a = Value { n: 42 }; assert_eq!((|v: Value| -> i32 { v.n })(a), 42); }",
                expect: Expect::Run(""),
            },
            Case {
                name: "mut_parameter_and_loop",
                source: "fn main() { assert_eq!((|mut n: i32| -> i32 { loop { if n == 0 { break 42; } n -= 1; } })(3), 42); }",
                expect: Expect::Run(""),
            },
            Case {
                name: "unused_enclosing_type_error",
                source: "fn unused() { (|x: i32| -> i32 { true })(1); } fn main() {}",
                expect: Expect::Invalid("E0308"),
            },
            Case {
                name: "unused_enclosing_ownership_error",
                source: "struct Value { n: i32 } fn unused() { (|v: Value| -> i32 { let moved = v; v.n })(Value { n: 1 }); } fn main() {}",
                expect: Expect::Invalid("E0382"),
            },
            Case {
                name: "double_move_argument",
                source: "struct Value { n: i32 } fn main() { let v = Value { n: 1 }; (|a: Value, b: Value| -> i32 { a.n + b.n })(v, v); }",
                expect: Expect::Invalid("E0382"),
            },
            Case {
                name: "argument_loan_conflict",
                source: "fn main() { let mut n = 1i32; (|a: &mut i32, b: &i32| -> i32 { *a + *b })(&mut n, &n); }",
                expect: Expect::Invalid("E0502"),
            },
            Case {
                name: "body_loan_conflict",
                source: "fn main() { (|mut n: i32| -> i32 { let a = &n; n = 2; *a })(1); }",
                expect: Expect::Invalid("E0506"),
            },
        ],
    );
}

#[test]
fn shared_scalar_parameter_captures_match_stock() {
    assert!(
        std::process::Command::new("rustc")
            .arg("-vV")
            .output()
            .is_ok_and(|out| out.status.success()),
        "local stock oracle required"
    );
    oracle::check(
        "closure-captures",
        &[
            Case {
                name: "shared_parameter",
                source: "fn run(n:i32)->i32{(||->i32{n+n})()}fn main(){assert_eq!(run(21),42);}",
                expect: Expect::Run(""),
            },
            Case {
                name: "multiple_first_use_and_dedup",
                source: "fn run(a:i32,b:i32)->i32{(||->i32{b+a+b})()}fn main(){assert_eq!(run(2,20),42);}",
                expect: Expect::Run(""),
            },
            Case {
                name: "closure_local_shadow",
                source: "fn run(n:i32)->i32{(||->i32{let out=n;let n=2;out+n})()}fn main(){assert_eq!(run(40),42);}",
                expect: Expect::Run(""),
            },
            Case {
                name: "capture_last_use",
                source: "fn run(mut n:i32)->i32{let out=(||->i32{n})();n+=1;out+n}fn main(){assert_eq!(run(20),41);}",
                expect: Expect::Run(""),
            },
            Case {
                name: "capture_and_argument_order_once",
                source: "fn mark(n:i32)->i32{println!(\"{}\",n);n}fn run(n:i32)->i32{(|x:i32,y:i32|->i32{n+x+y})(crate::mark(1),crate::mark(2))}fn main(){assert_eq!(run(39),42);}",
                expect: Expect::Run("1\n2\n"),
            },
            Case {
                name: "shared_capture_place",
                source: "fn read(n:&i32)->i32{*n}fn run(n:i32)->i32{(||->i32{crate::read(&n)})()}fn main(){assert_eq!(run(42),42);}",
                expect: Expect::Run(""),
            },
            Case {
                name: "nested_capture_environments",
                source: "fn run(n:i32)->i32{(|x:i32|->i32{n+(||->i32{x})()})(2)}fn main(){assert_eq!(run(40),42);}",
                expect: Expect::Run(""),
            },
            Case {
                name: "unused_captured_type_error",
                source: "fn unused(n:i32){(||->i32{let x=n;true})();}fn main(){}",
                expect: Expect::Invalid("E0308"),
            },
            Case {
                name: "unused_captured_move_error",
                source: "struct Value{n:i32}fn unused(n:i32){(|v:Value|->i32{let moved=v;n+v.n})(Value{n:1});}fn main(){}",
                expect: Expect::Invalid("E0382"),
            },
            Case {
                name: "capture_loan_argument_write",
                source: "fn run(mut n:i32){(|x:()|->i32{n})({n=2;});}fn main(){run(1);}",
                expect: Expect::Invalid("E0506"),
            },
            Case {
                name: "capture_loan_argument_borrow",
                source: "fn run(mut n:i32){(|x:&mut i32|->i32{n})(&mut n);}fn main(){run(1);}",
                expect: Expect::Invalid("E0502"),
            },
        ],
    );
}

#[test]
fn unsupported_closure_values_and_inference_fail_explicitly() {
    for source in [
        "fn main() { let f = |x: i32| -> i32 { x }; }",
        "fn main() { let f = (|x: i32| -> i32 { x }); }",
        "fn main() { (|x: i32| { x })(1); }",
        "fn main() { (|x| -> i32 { x })(1); }",
        "fn main() { (|x: &i32| -> &i32 { x })(&1); }",
        "fn global(x: i32) -> i32 { x } fn main() { let global = 1; (|| -> i32 { global(1) })(); }",
        "fn main() { let n = 1i32; (|| -> i32 { n })(); }",
        "struct Value; fn main() { let Value = 1i32; (|| -> i32 { Value })(); }",
    ] {
        for workers in [1, 8] {
            let failure = compiler::compile_observed(source.into(), workers, false)
                .err()
                .expect("unsupported closure must reject");
            assert!(
                failure.to_string().contains("unsupported"),
                "{failure}\n{source}"
            );
        }
    }
}

#[test]
fn anonymous_bodies_have_distinct_source_rooted_graph_identity() {
    let source = "fn main() { let a = (|| -> i32 { 1 })(); let b = (|| -> i32 { 2 })(); }";
    let parsed = compiler::frontend::parse(source).unwrap();
    let bodies: Vec<_> = parsed
        .functions
        .iter()
        .filter(|f| {
            f.name
                .segments()
                .last()
                .is_some_and(|n| parsed.names.spelling(*n).unwrap().starts_with("$closure$"))
        })
        .collect();
    assert_eq!(bodies.len(), 2);
    assert_ne!(
        bodies[0]
            .name
            .key(compiler::semantic_name::Namespace::Value),
        bodies[1]
            .name
            .key(compiler::semantic_name::Namespace::Value)
    );
    let spans: Vec<_> = bodies
        .iter()
        .map(|f| {
            parsed
                .names
                .span(*f.name.segments().last().unwrap())
                .unwrap()
        })
        .collect();
    assert_ne!(spans[0], spans[1]);
    let mut previous = None;
    for workers in [1, 8] {
        let compiled = compiler::compile(source.into(), workers, true).unwrap();
        assert_eq!(compiled.functions, 3);
        // Existing per-declaration template and ownership proof nodes, not an
        // extra whole-program closure phase or a frontend-expanded body.
        for kind in [110, 112] {
            let keys: std::collections::BTreeSet<_> = compiled
                .report
                .node_evaluations
                .iter()
                .filter(|(key, _)| key.kind == kind)
                .map(|(key, _)| key.params.clone())
                .collect();
            assert!(
                keys.len() >= 3,
                "missing independent closure callable nodes: {keys:?}"
            );
            if kind == 110 {
                if let Some(expected) = &previous {
                    assert_eq!(&keys, expected, "worker count changed callable identity");
                }
                previous = Some(keys);
            }
        }
    }
}

#[test]
fn closure_nesting_has_a_live_per_task_depth_gate() {
    let mut body = "1".to_owned();
    for _ in 0..140 {
        body = format!("(|| -> i32 {{ {body} }})()");
    }
    let source = format!("fn main() {{ let n = {body}; }}");
    let failure = compiler::compile_observed(source, 1, true)
        .err()
        .expect("depth must be bounded");
    assert!(!failure.to_string().contains("panicked"));
    assert!(
        failure.to_string().contains("limit") || failure.to_string().contains("depth"),
        "{failure}"
    );
    let source = format!("fn main() {{ {} }}", "(|| -> i32 { 1 })();".repeat(32));
    compiler::compile(source, 8, true).expect("sibling closures restore live depth");
}
