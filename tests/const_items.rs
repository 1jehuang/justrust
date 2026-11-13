//! General const/static items, const fn, inline const blocks and const-expression
//! array lengths: native run output matches stock rustc semantics.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#![allow(clippy::duplicate_mod)]
use justrust_engine::compiler;
#[path = "support/stock_rustc.rs"]
mod stock_rustc;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

fn run(source: &str, workers: usize, tag: &str) -> String {
    compiler::compile_observed(source.into(), workers, true)
        .unwrap_or_else(|e| panic!("check {tag}: {e}"));
    let compiled = compiler::compile_observed(source.into(), workers, false)
        .unwrap_or_else(|e| panic!("build {tag}: {e}"));
    let dir = std::env::temp_dir().join(format!(
        "justrust-const-items-{}-{tag}-{workers}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).unwrap();
    let binary = dir.join("program");
    fs::write(&binary, compiled.executable.unwrap()).unwrap();
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
    let output = Command::new(&binary).output().unwrap();
    assert!(output.status.success(), "{tag}: {:?}", output.status);
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn general_constants_match_stock_output() {
    let cases = [
        (
            "static",
            "static S: i32 = 7; fn main(){ println!(\"{}\", S); }",
            "7\n",
        ),
        (
            "constfn",
            "const fn sq(x: i32) -> i32 { x * x } const Q: i32 = sq(4); fn main(){ println!(\"{} {}\", Q, sq(3)); }",
            "16 9\n",
        ),
        (
            "static_refs",
            "mod m { pub static T: [i64; 2] = [-4, 9]; pub static B: bool = true; }
             static S: i32 = 5; static C: char = 'x';
             static N: [[u16; 2]; 2] = [[1, 2], [3, 4]]; static Z: [u8; 4] = [7; 4];
             fn get() -> &'static i32 { &S }
             fn first() -> &'static [i64; 2] { &m::T }
             fn main(){
                 let r = &S;
                 println!(\"{} {} {}\", r, std::ptr::eq(&S, &S), std::ptr::eq(get(), r));
                 let t = &m::T; let b = &m::B; let c = &C;
                 println!(\"{:?} {} {} {}\", t, b, c, first()[1] == t[1]);
                 let s: &[u8] = &Z; let n = &N;
                 println!(\"{:?} {} {}\", s, s.len(), n[1][0] + N[0][1]);
             }",
            "5 true true\n[-4, 9] true x true\n[7, 7, 7, 7] 4 5\n",
        ),
        (
            "aggregate",
            "const A: [i32; 3] = [1, 2, 3]; struct P { x: i32 } const O: P = P { x: 3 }; fn main(){ let v = A[1]; let o = O.x; println!(\"{} {}\", v, o); }",
            "2 3\n",
        ),
        (
            "str",
            "const S: &str = \"hi\"; fn main(){ println!(\"{}\", S); }",
            "hi\n",
        ),
        (
            "inline",
            "fn main(){ let x = const { 3 + 4 }; println!(\"{}\", x); }",
            "7\n",
        ),
        (
            "arraylen",
            "const N: usize = 3; const M: usize = N * 2 + 1; fn main(){ let a = [5u8; M]; let b: [i32; N * 2] = [1, 2, 3, 4, 5, 6]; println!(\"{} {}\", a[6], b[5]); }",
            "5 6\n",
        ),
    ];
    for workers in [1, 8] {
        for (tag, source, expected) in cases {
            assert_eq!(run(source, workers, tag), expected, "{tag}");
        }
    }
}

#[test]
fn invalid_constants_reject_even_when_unused() {
    for (source, category) in [
        // Immutable statics are places with one address, never writable.
        (
            "static S: i32 = 1; fn main(){ let r = &mut S; *r = 2; }",
            "E0596",
        ),
        (
            "static S: i32 = 1; fn f() -> &'static mut i32 { &mut S } fn main(){}",
            "E0596",
        ),
        (
            "static A: [i32; 2] = [1, 2]; fn main(){ let r = &mut A[0]; *r = 3; }",
            "E0596",
        ),
        ("static S: i32 = 1; fn main(){ S = 2; }", "E0594"),
        ("static S: i32 = 1; fn main(){ S += 2; }", "E0594"),
        (
            "static A: [i32; 2] = [1, 2]; fn main(){ A[0] = 3; }",
            "E0594",
        ),
        ("static S: i32 = 1; fn main(){ let _r: &u8 = &S; }", "E0308"),
        // Borrowed statics outside the closed scalar/array subset fail closed.
        (
            "static S: &str = \"x\"; fn main(){ let r = &S; println!(\"{}\", r); }",
            "unsupported",
        ),
        (
            "fn f()->[i32;1]{[1]} const A:[i32;1]=f(); fn main(){}",
            "E0015",
        ),
        ("const A:[i32;2]=[1,true]; fn main(){}", ""),
        // Mutable static storage is explicitly unsupported, including unsafe access.
        (
            "static mut X:u8=1;fn main(){ X = 2; }",
            "mutable static storage",
        ),
    ] {
        let error = compiler::compile_observed(source.into(), 1, true)
            .err()
            .unwrap_or_else(|| panic!("accepted {source}"));
        assert!(error.to_string().contains(category), "{source}: {error}");
    }
}

#[test]
fn unused_const_bodies_check_all_calls_before_codegen() {
    let dir = std::env::temp_dir().join(format!("justrust-const-legality-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    for (index, body) in [
        "runtime()",
        "let x=runtime(); x",
        "if false { runtime() } else { 0 }",
        "match 0 { 0 => 0, _ => runtime() }",
        "let x={ runtime(); 0 }; x",
    ]
    .iter()
    .enumerate()
    {
        let source =
            format!("fn runtime()->i32{{7}} const fn invalid()->i32{{{body}}} fn main(){{}}");
        let input = dir.join(format!("invalid{index}.rs"));
        fs::write(&input, &source).unwrap();
        let stock = Command::new(stock_rustc::stock_rustc())
            .args(["--edition=2024", "--emit=metadata"])
            .arg(&input)
            .arg("-o")
            .arg(dir.join("stock.rmeta"))
            .output()
            .unwrap();
        assert!(!stock.status.success(), "stock accepted {source}");
        assert!(String::from_utf8_lossy(&stock.stderr).contains("E0015"));
        for workers in [1, 8] {
            for check in [true, false] {
                let failure = compiler::compile_observed(source.clone(), workers, check)
                    .err()
                    .unwrap_or_else(|| {
                        panic!("accepted workers={workers} check={check}: {source}")
                    });
                assert!(failure.to_string().contains("E0015"), "{failure}");
                if let Some(report) = failure.report {
                    assert!(report.capture_complete);
                    assert!(
                        !report.published_kind(103) && !report.published_kind(105),
                        "invalid body published native object generation"
                    );
                }
            }
        }
    }
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn public_program_builder_cannot_skip_const_call_qualification() {
    use compiler::frontend::{self, Block, Constness, Expr, ProgramBuilder};
    let prototype = frontend::parse("fn f(){}fn main(){}").unwrap();
    for missing in [false, true] {
        let mut builder = ProgramBuilder::new(
            std::path::Path::new("public-const.rs"),
            "",
            &Default::default(),
        )
        .unwrap();
        let module = builder.module_root().unwrap();
        let target = builder.path("crate::runtime").unwrap();
        let mut invalid = prototype.functions[0].clone();
        invalid.name = builder.path("crate::invalid").unwrap();
        invalid.module = module;
        invalid.visibility = compiler::ir::Visibility::Public;
        invalid.lexical_scope = None;
        invalid.constness = Constness::Const;
        invalid.body = Block {
            statements: Vec::new(),
            tail: Some(Box::new(Expr::Call {
                name: target.clone(),
                args: Vec::new(),
            })),
        };
        if !missing {
            let mut runtime = prototype.functions[0].clone();
            runtime.name = target;
            runtime.module = module;
            runtime.visibility = compiler::ir::Visibility::Public;
            runtime.lexical_scope = None;
            builder.functions.push(runtime);
        }
        builder.functions.push(invalid);
        let mut main = prototype.functions[1].clone();
        main.name = builder.path("crate::main").unwrap();
        main.module = module;
        main.visibility = compiler::ir::Visibility::Public;
        main.lexical_scope = None;
        builder.functions.push(main);
        let program = builder.finish().unwrap();
        let error = frontend::check(&program).unwrap_err();
        assert!(
            error.contains(if missing {
                "unresolved callable"
            } else {
                "E0015"
            }),
            "{error}"
        );
    }
}

#[test]
fn const_foreign_calls_and_output_are_rejected() {
    let dir = std::env::temp_dir().join(format!("justrust-const-effects-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    for (index, source) in [
        "unsafe extern \"C\" {fn foreign()->i32;} const fn invalid()->i32{unsafe{foreign()}} fn main(){}",
        "const fn invalid(){println!(\"not const\");} fn main(){}",
        "const fn invalid(x:i32){println!(\"{}\",x);} fn main(){}",
        "use std::ops::Add;struct A{v:i32}impl Add for A{type Output=i32;fn add(self,other:A)->i32{self.v+other.v}} const fn invalid()->i32{A{v:1}+A{v:2}} fn main(){}",
    ].iter().enumerate() {
        let input = dir.join(format!("effect{index}.rs"));
        fs::write(&input, source).unwrap();
        let stock = Command::new(stock_rustc::stock_rustc())
            .args(["--edition=2024", "--emit=metadata"])
            .arg(input).arg("-o").arg(dir.join("stock.rmeta"))
            .output().unwrap();
        assert!(!stock.status.success());
        assert!(String::from_utf8_lossy(&stock.stderr).contains("E0015"));
        for workers in [1, 8] {
            for check in [true, false] {
                let error = compiler::compile_observed((*source).into(), workers, check).err().expect("const effect was accepted");
                assert!(error.to_string().contains("E0015"), "{source}: {error}");
                if let Some(report) = error.report {
                    assert!(!report.published_kind(105), "{source}: failed const check published BUILD");
                    if check {
                        assert!(!report.node_evaluations.iter().any(|(key, _)| key.kind == 103), "{source}: CHECK demanded OBJECT");
                    }
                }
            }
        }
    }
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn unused_const_generic_destructors_are_not_runtime_only_checks() {
    let dir = std::env::temp_dir().join(format!("justrust-const-drop-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    for (index, source) in [
        "const fn discard<T>(value:T){} fn main(){}",
        "const fn keep_first<T>(a:T,b:T)->T{a} fn main(){}",
        "const fn replace<T>(mut a:T,b:T)->T{a=b;a} fn main(){}",
    ]
    .iter()
    .enumerate()
    {
        let input = dir.join(format!("invalid{index}.rs"));
        fs::write(&input, source).unwrap();
        let stock = Command::new(stock_rustc::stock_rustc())
            .args(["--edition=2024", "--emit=metadata"])
            .arg(input)
            .arg("-o")
            .arg(dir.join("stock.rmeta"))
            .output()
            .unwrap();
        assert!(!stock.status.success());
        assert!(String::from_utf8_lossy(&stock.stderr).contains("E0493"));
        for workers in [1, 8] {
            for check in [true, false] {
                let error = compiler::compile_observed((*source).into(), workers, check)
                    .err()
                    .expect("unused const destructor was accepted");
                assert!(error.to_string().contains("E0493"), "{error}");
            }
        }
    }
    for (index, source) in [
        "const fn identity<T>(value:T)->T{value} fn main(){let x=identity(42);println!(\"{}\",x);}",
        "const fn forward<T>(value:T)->T{let moved=value;moved} fn main(){let x=forward(42);println!(\"{}\",x);}",
        "const fn discard<T:Copy>(value:T){} fn main(){discard(42);println!(\"{}\",42);}",
    ].iter().enumerate() {
        let input = dir.join(format!("valid{index}.rs"));
        fs::write(&input, source).unwrap();
        let stock = Command::new(stock_rustc::stock_rustc())
            .args(["--edition=2024", "--emit=metadata"])
            .arg(&input).arg("-o").arg(dir.join("stock-valid.rmeta"))
            .output().unwrap();
        assert!(stock.status.success(), "{source}: {}", String::from_utf8_lossy(&stock.stderr));
        for workers in [1, 8] {
            assert_eq!(run(source, workers, "const-copy-or-forward"), "42\n");
        }
    }
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn valid_const_calls_and_literal_panic_keep_stock_behavior() {
    let source = "mod helper {pub const fn identity(x:i32)->i32{x}} use helper::identity as renamed; const fn value(x:i32)->i32{if false {panic!(\"unreachable\")} else {renamed(x)+1}} const ANSWER:i32=value(41); fn main(){println!(\"{}\", ANSWER);}";
    let dir = std::env::temp_dir().join(format!("justrust-const-valid-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let direct_panic = "const fn never()->i32{panic!(\"literal panic\")} fn main(){}";
    let panic_input = dir.join("direct-panic.rs");
    fs::write(&panic_input, direct_panic).unwrap();
    let stock_panic = Command::new(stock_rustc::stock_rustc())
        .args(["--edition=2024", "--emit=metadata"])
        .arg(&panic_input)
        .arg("-o")
        .arg(dir.join("panic.rmeta"))
        .output()
        .unwrap();
    assert!(
        stock_panic.status.success(),
        "{}",
        String::from_utf8_lossy(&stock_panic.stderr)
    );
    for workers in [1, 8] {
        for check in [true, false] {
            match compiler::compile_observed(direct_panic.into(), workers, check) {
                Ok(compiled) => assert_eq!(compiled.executable.is_none(), check),
                Err(error) => {
                    let diagnostic = error.to_string();
                    assert!(!diagnostic.contains("E0015"), "{diagnostic}");
                    assert!(diagnostic.contains("unsupported"), "{diagnostic}");
                    if let Some(report) = error.report {
                        assert!(report.capture_complete);
                        assert!(!report.published_kind(105));
                    }
                }
            }
        }
    }
    let input = dir.join("valid.rs");
    let binary = dir.join("stock");
    fs::write(&input, source).unwrap();
    let stock = Command::new(stock_rustc::stock_rustc())
        .arg("--edition=2024")
        .arg(input)
        .arg("-o")
        .arg(&binary)
        .output()
        .unwrap();
    assert!(
        stock.status.success(),
        "{}",
        String::from_utf8_lossy(&stock.stderr)
    );
    let stock = Command::new(binary).output().unwrap();
    assert!(stock.status.success());
    for workers in [1, 8] {
        assert_eq!(
            run(source, workers, "qualified-const"),
            String::from_utf8_lossy(&stock.stdout)
        );
    }
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn block_local_const_items_match_stock() {
    #[path = "support/closure_oracle.rs"]
    mod closure_oracle;
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "block-local-consts",
        &[
            Case {
                name: "scoping_shadowing_order_and_array_len",
                source: r#"const C: u32 = 1;
fn adler(a: u32, n: usize) -> u32 { const MOD: u32 = 65521; const CHUNK_SIZE: usize = 5552 * 4; let b = (n % CHUNK_SIZE) as u32; (a + b) % MOD }
fn sibling() -> u32 { const A: u32 = B * 2; const B: u32 = 4; A }
fn a() -> u32 { const K: u32 = 10; K }
fn b() -> u32 { const K: u32 = 20; K }
struct S;
impl S { fn g(&self) -> u32 { const K: u32 = 6; K } }
mod algo {
    pub struct U(pub u32);
    impl U { pub fn m(&self) -> u32 { const MOD: u32 = 65521; self.0 % MOD } }
    pub fn n() -> u32 { const C: u32 = 3; C }
}
fn main() {
    let early = L;
    const L: u32 = 4;
    const N: usize = 3;
    let arr = [7u8; N];
    let outer = { const C: u32 = 2; C };
    println!("{} {} {} {} {} {} {} {} {} {} {}", C, outer, early, adler(70000, 22210), sibling(), a(), b(), S.g(), arr.len(), algo::U(70000).m(), algo::n());
}"#,
                expect: Expect::Run("1 2 4 4481 8 10 20 6 3 4479 3\n"),
            },
            Case {
                name: "out_of_scope",
                source: "fn main(){ { const C: u32 = 3; } println!(\"{}\", C); }",
                expect: Expect::Invalid("E0425"),
            },
            Case {
                name: "initializer_reads_local",
                source: "fn main(){ let x = 7u32; const C: u32 = x; }",
                expect: Expect::Invalid("E04"),
            },
            Case {
                name: "duplicate_in_block",
                source: "fn main(){ const C: u32 = 1; const C: u32 = 2; println!(\"{}\", C); }",
                expect: Expect::Invalid("E0428"),
            },
            Case {
                name: "let_binding_names_const",
                source: "fn main(){ const C: u32 = 5; { let C = 9u32; println!(\"{}\", C); } }",
                expect: Expect::Invalid("E0005"),
            },
        ],
    );
}

#[test]
fn block_local_fn_and_use_items_match_stock() {
    #[path = "support/closure_oracle.rs"]
    mod closure_oracle;
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "block-local-items",
        &[
            Case {
                name: "imports_functions_shadowing_and_nesting",
                source: r#"mod m { pub fn three() -> u32 { 3 } pub struct P(pub u32); pub fn a() -> u32 { 2 } }
fn helper() -> u32 { 100 }
fn main() {
    use m::three;
    use m::P as Q;
    fn helper() -> u32 { 7 }
    fn twice(x: u32) -> u32 { x * 2 + inner() }
    fn inner() -> u32 { 1 }
    let q = Q(4);
    println!("{} {} {} {}", three(), helper(), twice(q.0), crate::helper());
    {
        fn helper() -> u32 { 9 }
        println!("{}", helper());
    }
    println!("{}", early());
    fn early() -> u32 { 5 }
    use m::a as b;
    fn c() -> u32 { b() + 1 }
    println!("{} {}", helper(), c());
}"#,
                expect: Expect::Run("3 7 9 100\n9\n5\n7 3\n"),
            },
            Case {
                name: "fn_reads_enclosing_local",
                source: "fn main(){ let x = 1; fn f() -> i32 { x } println!(\"{}\", f()); }",
                expect: Expect::Invalid("E0434"),
            },
            Case {
                name: "fn_uses_enclosing_generic",
                source: "fn g<T: Default>() -> T { fn h() -> T { T::default() } h() } fn main(){}",
                expect: Expect::Invalid("E0401"),
            },
            Case {
                name: "fn_out_of_scope",
                source: "fn main(){ { fn f() -> u32 { 1 } } println!(\"{}\", f()); }",
                expect: Expect::Invalid("E0425"),
            },
            Case {
                name: "duplicate_fn_in_block",
                source: "fn main(){ fn f() {} fn f() {} }",
                expect: Expect::Invalid("E0428"),
            },
            Case {
                name: "unused_invalid_fn_body",
                source: "fn main(){ fn bad() -> u32 { \"s\" } }",
                expect: Expect::Invalid("E0308"),
            },
            Case {
                name: "unused_unresolved_use",
                source: "fn main(){ use std::nope::X; }",
                expect: Expect::Invalid("E0432"),
            },
            Case {
                name: "unused_private_use",
                source: "mod m { fn private() {} } fn main(){ use m::private; }",
                expect: Expect::Invalid("E0603"),
            },
        ],
    );
}

#[test]
fn block_local_nominal_items_match_stock() {
    #[path = "support/closure_oracle.rs"]
    mod closure_oracle;
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "block-local-nominals",
        &[
            Case {
                name: "structs_enums_traits_impls_aliases",
                source: r#"use std::fmt;
fn make(n: i32) -> i32 { struct Acc { n: i32 } let a = Acc { n }; a.n + 1 }
fn main() {
    let r = make(3);
    enum Bar { U(i32), W }
    struct P { x: i32 }
    impl P { fn twice(&self) -> i32 { self.x * 2 } }
    trait Shape { fn area(&self) -> i32; }
    impl Shape for P { fn area(&self) -> i32 { self.x * self.x } }
    impl fmt::Display for P { fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result { write!(f, "P({})", self.x) } }
    const C: i32 = 5;
    type T = i32;
    let b = Bar::U(3);
    let v: T = match b { Bar::U(a) => a, Bar::W => 0 };
    let p = P { x: v + C };
    println!("{} {} {} {}", p.twice(), p.area(), p, r);
    {
        struct P(u8);
        let q = P(7);
        println!("{}", q.0);
    }
}"#,
                expect: Expect::Run("16 64 P(8) 4\n7\n"),
            },
            Case {
                name: "struct_uses_enclosing_generic",
                source: "fn f<T>(x: T) { struct S { v: T } } fn main(){}",
                expect: Expect::Invalid("E0401"),
            },
            Case {
                name: "method_reads_enclosing_local",
                source: "fn main(){ let y = 3; struct S; impl S { fn g(&self) -> i32 { y } } }",
                expect: Expect::Invalid("E0434"),
            },
            Case {
                name: "duplicate_struct_in_block",
                source: "fn main(){ struct S; struct S; }",
                expect: Expect::Invalid("E0428"),
            },
            Case {
                name: "struct_out_of_scope",
                source: "fn main(){ { struct Q; } let _q = Q; }",
                expect: Expect::Invalid("E0425"),
            },
        ],
    );
}

#[test]
fn unnamed_const_items_match_stock() {
    #[path = "support/closure_oracle.rs"]
    mod closure_oracle;
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "unnamed-consts",
        &[
            Case {
                name: "module_and_block_unnamed_consts",
                source: r#"const _: () = ();
const _: u32 = 3 + 4;
const N: usize = 4;
const _: () = assert!(N > 2);
fn f() -> u32 { const _: () = assert!(1 < 2); const _: u8 = 1; 5 }
fn main() {
    let x = unsafe { const _: () = assert!(4 <= 8); 7u32 };
    println!("{} {}", f(), x);
}"#,
                expect: Expect::Run("5 7\n"),
            },
            Case {
                name: "unnamed_const_assert_fails",
                source: "const _: () = assert!(1 > 2); fn main(){}",
                expect: Expect::Invalid("E0080"),
            },
            Case {
                name: "unnamed_const_type_mismatch",
                source: "fn main(){ const _: u8 = true; }",
                expect: Expect::Invalid("E0308"),
            },
            Case {
                name: "block_unnamed_const_assert_fails",
                source: "fn main(){ const _: () = assert!(1 > 2); }",
                expect: Expect::Invalid("E0080"),
            },
            Case {
                name: "unnamed_const_is_not_a_name",
                source: "const _: u32 = 1; fn main(){ let _x: u32 = _; }",
                expect: Expect::Invalid("error"),
            },
        ],
    );
}

#[test]
fn unevaluated_constant_initializers_fail_closed() {
    #[path = "support/closure_oracle.rs"]
    mod closure_oracle;
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "unevaluated-consts",
        &[
            Case {
                name: "aggregate_and_reference_constants_still_work",
                source: r#"struct P { x: i32, y: [u8; 2] }
const ORIGIN: P = P { x: -3, y: [1, 2] };
const NAME: &str = "jr";
const PAIR: (u8, char) = (7, 'z');
const F: f64 = 1.5;
fn main() { println!("{} {} {} {} {} {}", ORIGIN.x, ORIGIN.y[1], NAME, PAIR.0, PAIR.1, F); }"#,
                expect: Expect::Run("-3 2 jr 7 z 1.5\n"),
            },
            Case {
                name: "unused_block_panic_constant",
                source: "const X: u32 = { panic!(\"no\") }; fn main(){}",
                expect: Expect::Invalid("E0080"),
            },
        ],
    );
}
