//! Original root-arm binder-free OR M1. Stock32 plus separately approved I9.
//! Stock 1.98.1 48a229ceaefd4985c50990b14116b6d856af0985, edition2024.
//! Policy exclusions are valid Rust, not invalid-language diagnostics.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
use justrust_engine::compiler;
use std::{fs, os::unix::fs::PermissionsExt, process::Command};
const CASES: &[(&str, &str)] = &[
    (
        "r01_integer",
        "fn main(){let n=2u8;let x=match n{1|2=>7,_=>0};println!(\"{}\",x);}",
    ),
    (
        "r02_bool",
        "fn main(){let x=match true{true|false=>7};println!(\"{}\",x);}",
    ),
    (
        "r03_char",
        "fn main(){let x=match '['{'('|'['|'{'=>7,_=>0};println!(\"{}\",x);}",
    ),
    (
        "r04_tuple",
        "fn main(){let x=match ('[',']'){('(',')')|('[',']')|('{','}')=>7,_=>0};println!(\"{}\",x);}",
    ),
    (
        "r05_tuple_partition",
        "fn main(){let x=match(false,true){(true,_)|(false,_)=>7};println!(\"{}\",x);}",
    ),
    (
        "r06_signed",
        "fn main(){let x=match -128i8{-128|127=>7,_=>0};println!(\"{}\",x);}",
    ),
    (
        "r07_matches",
        "fn main(){println!(\"{}\",matches!(('[',']'),('(',')')|('[',']')|('{','}')));}",
    ),
    (
        "r08_fallback_once",
        "fn main(){let mut n=0i32;let x=match 3u8{1|2=>{n+=1;7},_=>{n+=10;0}};println!(\"{} {}\",x,n);}",
    ),
    (
        "r09_source_body_once",
        "fn take(n:&mut i32)->(char,char){*n+=1;('[',']')}fn main(){let mut n=0i32;let mut b=0i32;let x=match take(&mut n){('[',')')|('[',']')=>{b+=1;7},_=>0};println!(\"{} {} {}\",x,n,b);}",
    ),
    (
        "r10_unit_tuple",
        "fn main(){let x=match((),false){((),true)|((),false)=>7};println!(\"{}\",x);}",
    ),
    (
        "i01_nonexhaustive",
        "fn main(){let _=match(false,false){(true,_)|(false,true)=>1};}",
    ),
    (
        "i02_leaf_type",
        "fn main(){let _=match 0u8{0|true=>1,_=>2};}",
    ),
    (
        "i03_tuple_shape",
        "fn main(){let _=match(0u8,1u8){(0,1)|(0,1,2)=>1,_=>0};}",
    ),
    (
        "i04_literal_overflow",
        "fn main(){let _=match 0u8{0|256=>1,_=>0};}",
    ),
    (
        "i05_unused_body_type",
        "fn bad(x:u8)->i32{match x{0|1=>true,_=>0}}fn main(){}",
    ),
    (
        "i06_dead_body_name",
        "fn bad(x:bool)->i32{match x{_=>0,true|false=>missing}}fn main(){}",
    ),
    (
        "i07_missing_qualified",
        "fn main(){let _=match 0u8{0|Missing::VALUE=>1,_=>0};}",
    ),
    (
        "i08_missing_alternative",
        "fn main(){let _=match 0u8{0|=>1,_=>0};}",
    ),
    ("p01_duplicate", "fn main(){let _=match 0u8{0|0=>1,_=>0};}"),
    (
        "p02_subsumed",
        "fn main(){let _=match 1u8{0=>0,0|1=>1,_=>2};}",
    ),
    (
        "p03_nested",
        "fn main(){let _=match (0u8,1u8){(0|1,_)=>1,_=>0};}",
    ),
    ("p04_binding", "fn main(){let _=match 1u8{x|x=>x};}"),
    (
        "p05_match_guard",
        "fn main(){let _=match 1u8{0|1 if true=>1,_=>0};}",
    ),
    (
        "p06_matches_guard",
        "fn main(){let _=matches!(1u8,0|1 if true);}",
    ),
    (
        "p07_reference",
        "fn main(){let _=match &1u8{&0|&1=>1,_=>0};}",
    ),
    (
        "p08_noncopy",
        "struct S(u8);fn main(){let s=S(0);let _=match s{S(0)|S(1)=>0,_=>1};}",
    ),
    (
        "p09_parameter",
        "fn f((true|false):bool){}fn main(){f(true);}",
    ),
    ("p10_let", "fn main(){let (true|false)=true;}"),
    ("p11_for", "fn main(){for (true|false) in [true,false]{}}"),
    (
        "p12_generated_match",
        "macro_rules! m{()=>{match 1u8{0|1=>7,_=>0}}}fn main(){println!(\"{}\",m!());}",
    ),
    (
        "p13_generated_matches",
        "macro_rules! m{()=>{matches!(1u8,0|1)}}fn main(){println!(\"{}\",m!());}",
    ),
    ("p14_at", "fn main(){let _=match 1u8{x @ (0|1)=>x,_=>0};}"),
    (
        "i09_missing_alternative_spaced",
        "fn main() { let _ = match 0 { 0 | => 7, _ => 0 }; }\n",
    ),
];

#[test]
fn exact_stock_sources_check_build_and_execute_workers1_8() {
    for workers in [1, 8] {
        for &(name, source) in CASES.iter().filter(|(name, _)| name.starts_with('r')) {
            let checked = compiler::compile_observed(source.into(), workers, true)
                .unwrap_or_else(|e| panic!("{name} CHECK w{workers}: {e}"));
            assert!(!checked.report.published_kind(103));
            let built = compiler::compile_observed(source.into(), workers, false)
                .unwrap_or_else(|e| panic!("{name} BUILD w{workers}: {e}"));
            let base = std::env::var_os("JCODE_SCRATCH_DIR")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(std::env::temp_dir);
            let dir = base.join(format!(
                "jr-root-or-{}-{workers}-{name}",
                std::process::id()
            ));
            fs::create_dir(&dir).unwrap();
            let binary = dir.join("native");
            fs::write(&binary, built.executable.unwrap()).unwrap();
            fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
            let output = Command::new(&binary).output().unwrap();
            assert!(output.status.success(), "{name}: {:?}", output);
            let expected = match name {
                "r07_matches" => "true\n",
                "r08_fallback_once" => "0 10\n",
                "r09_source_body_once" => "7 1 1\n",
                _ => "7\n",
            };
            assert_eq!(
                String::from_utf8(output.stdout).unwrap(),
                expected,
                "{name} w{workers}"
            );
            fs::remove_dir_all(dir).unwrap();
        }
    }
}

#[test]
fn invalid_and_out_of_scope_rows_never_publish_build() {
    for workers in [1, 8] {
        for &(name, source) in CASES.iter().filter(|(name, _)| !name.starts_with('r')) {
            let bad_root = if matches!(name, "i05_unused_body_type" | "i06_dead_body_name") {
                let program = std::sync::Arc::new(compiler::frontend::parse(source).unwrap());
                let budget = compiler::ir::PlanBudget::default();
                let plan =
                    compiler::frontend::PlanCtx::from_shared_program(program, &budget).unwrap();
                let bad = (0..plan.callable_count())
                    .find(|&index| {
                        plan.declaration(plan.callable_decl(index).unwrap())
                            .unwrap()
                            .name
                            .rsplit("::")
                            .next()
                            == Some("bad")
                    })
                    .unwrap();
                Some(
                    plan.closed_roots()
                        .iter()
                        .position(|&index| index == bad)
                        .unwrap() as u64,
                )
            } else {
                None
            };
            for check in [true, false] {
                let error = compiler::compile_observed(source.into(), workers, check)
                    .err()
                    .unwrap_or_else(|| panic!("accepted {name} w{workers} check{check}"));
                let diagnostic = error.to_string().to_lowercase();
                if name == "p09_parameter" {
                    // Preexisting parameter grammar cannot parse this stock-valid
                    // pattern. Do not relabel it invalid Rust or broaden that grammar.
                    assert!(
                        diagnostic.contains("expected ascii identifier"),
                        "{name}: {error}"
                    );
                } else if name.starts_with('p') {
                    assert!(diagnostic.contains("unsupported"), "policy {name}: {error}");
                } else {
                    assert!(
                        !diagnostic.contains("unsupported"),
                        "invalid {name}: {error}"
                    );
                }
                assert!(
                    bad_root.is_none() || error.report.is_some(),
                    "{name} lost semantic graph report"
                );
                if let Some(report) = error.report {
                    assert!(report.capture_complete, "{name} incomplete graph capture");
                    assert!(!report.published_kind(105), "{name}");
                    if check {
                        assert!(!report.published_kind(103), "{name}");
                    }
                    if let Some(root) = bad_root {
                        for (node, (key, _)) in report.node_ids.iter().zip(&report.node_evaluations)
                        {
                            if key.kind == 103 && key.params == [root] {
                                assert!(
                                    !report.events.iter().any(|event| event.kind == "publish"
                                        && event.node == Some(*node)),
                                    "{name} published the offending unused callable object"
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}
