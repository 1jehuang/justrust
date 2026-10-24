//! General const/static items, const fn, inline const blocks and const-expression
//! array lengths: native run output matches stock rustc semantics.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
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
fn main() {
    let early = L;
    const L: u32 = 4;
    const N: usize = 3;
    let arr = [7u8; N];
    let outer = { const C: u32 = 2; C };
    println!("{} {} {} {} {} {} {} {} {}", C, outer, early, adler(70000, 22210), sibling(), a(), b(), S.g(), arr.len());
}"#,
                expect: Expect::Run("1 2 4 4481 8 10 20 6 3\n"),
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
