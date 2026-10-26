//! Trait objects and `impl Trait` (TRAIT-OBJECTS.md). `fixtures/trait_objects/
//! stock.tsv` and the `.stdout` files were recorded from stock rustc first
//! (`scripts/trait_objects_oracle.sh`). Native CHECK and BUILD run through the
//! public `jr-rust` CLI at workers 1 and 8:
//! - positive: stock accepts; native must accept and match stdout exactly.
//! - negative: stock rejects; native must reject (even unused code).
//! - boundary: stock accepts; native must fail with an explicit `unsupported`
//!   diagnostic and never build. Move a case to positive when it is supported.
//!
//! Stock rustc is never invoked by this test.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

const STOCK: &str = include_str!("fixtures/trait_objects/stock.tsv");
static NEXT: AtomicU64 = AtomicU64::new(0);

#[path = "support/native_prelude.rs"]
mod native_prelude;

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let root = std::env::var_os("JCODE_SCRATCH_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = root.join(format!("jr-trait-objects-{}-{id}", std::process::id()));
        fs::create_dir(&path).expect("exclusive scratch");
        Self(path)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/trait_objects")
}

struct Case {
    path: String,
    accept: bool,
}

fn cases() -> Vec<Case> {
    STOCK
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(|l| {
            let cols: Vec<&str> = l.split('\t').collect();
            assert_eq!(cols.len(), 3, "bad stock row {l:?}");
            Case {
                path: cols[0].into(),
                accept: match cols[1] {
                    "accept" => true,
                    "reject" => false,
                    other => panic!("bad stock verdict {other}"),
                },
            }
        })
        .collect()
}

fn native(source: &Path, output: Option<&Path>, workers: usize) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_jr-rust"));
    command
        .arg(source)
        .args(["--workers", &workers.to_string()]);
    match output {
        Some(path) => command.arg("-o").arg(path),
        None => command.arg("--check"),
    };
    command.output().expect("spawn jr-rust")
}

#[test]
fn unchanged_inventory_and_box_programs_match_stock_workers_1_and_8() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/full_programs");
    for name in ["20_inventory_system", "13_trait_objects"] {
        let source = root.join(format!("{name}.rs"));
        let expected = fs::read(root.join(format!("{name}.stdout"))).unwrap();
        for workers in [1, 8] {
            let scratch = Scratch::new();
            let exe = scratch.0.join("program");
            let check = native(&source, None, workers);
            assert!(
                check.status.success(),
                "{name} CHECK workers={workers}: {}",
                String::from_utf8_lossy(&check.stderr)
            );
            let build = native(&source, Some(&exe), workers);
            assert!(
                build.status.success(),
                "{name} BUILD workers={workers}: {}",
                String::from_utf8_lossy(&build.stderr)
            );
            let run = Command::new(&exe).output().expect("run unchanged program");
            assert!(run.status.success(), "{name} workers={workers}: {run:?}");
            assert_eq!(run.stdout, expected, "{name} workers={workers}");
            assert!(run.stderr.is_empty(), "{name} workers={workers}: {run:?}");
        }
    }
}

#[test]
fn stock_receipt_covers_every_fixture() {
    let listed: Vec<String> = cases().into_iter().map(|c| c.path).collect();
    for class in ["positive", "negative", "boundary"] {
        for entry in fs::read_dir(fixtures().join(class)).unwrap() {
            let name = entry.unwrap().file_name().into_string().unwrap();
            if let Some(stem) = name.strip_suffix(".rs") {
                let rel = format!("{class}/{stem}.rs");
                assert!(listed.contains(&rel), "{rel} missing from stock.tsv");
            }
        }
    }
    for case in cases() {
        let expect_accept = !case.path.starts_with("negative/");
        assert_eq!(
            case.accept, expect_accept,
            "{}: class disagrees with stock verdict",
            case.path
        );
        if case.accept {
            assert!(
                fixtures()
                    .join(case.path.replace(".rs", ".stdout"))
                    .exists(),
                "{}: missing stock stdout",
                case.path
            );
        }
    }
}

#[test]
fn native_matches_stock_workers_1_and_8() {
    let mut failures = Vec::new();
    for case in cases() {
        let source = fixtures().join(&case.path);
        for workers in [1, 8] {
            let scratch = Scratch::new();
            let exe = scratch.0.join("program");
            let check = native(&source, None, workers);
            let build = native(&source, Some(&exe), workers);
            let context = format!("{} workers={workers}", case.path);
            let stderr = String::from_utf8_lossy(&build.stderr).into_owned();
            if case.path.starts_with("positive/") {
                if !check.status.success() || !build.status.success() {
                    failures.push(format!(
                        "{context}: native rejected a supported program: {}{stderr}",
                        String::from_utf8_lossy(&check.stderr)
                    ));
                    continue;
                }
                let run = Command::new(&exe).output().expect("run executable");
                let expected =
                    fs::read(fixtures().join(case.path.replace(".rs", ".stdout"))).unwrap();
                if !run.status.success() || run.stdout != expected {
                    failures.push(format!(
                        "{context}: output differs from stock: {:?}",
                        String::from_utf8_lossy(&run.stdout)
                    ));
                }
            } else {
                if check.status.success() || build.status.success() || exe.exists() {
                    failures.push(format!("{context}: native accepted"));
                    continue;
                }
                if check.status.code() != Some(1) || build.status.code() != Some(1) {
                    failures.push(format!("{context}: rejection was not a diagnostic exit"));
                }
                if case.path.starts_with("boundary/") && !stderr.contains("unsupported") {
                    failures.push(format!(
                        "{context}: boundary must fail explicitly unsupported: {stderr}"
                    ));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn dyn_drop_owner_preserves_all_body_proofs_and_object_gates() {
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(|| {
            let source = include_str!("fixtures/trait_objects/positive/dyn_drop_owner.rs");
            for workers in [1, 8] {
                for check_only in [true, false] {
                    let result =
                        justrust_engine::compiler::compile(source.to_owned(), workers, check_only)
                            .unwrap();
                    assert_eq!(result.functions, 4);
                    assert_eq!(result.analyses.len(), result.functions);
                    assert!(result.analyses.iter().all(|analysis| analysis.passed()));
                    assert_eq!(result.executable.is_none(), check_only);
                    let report = result.report;
                    assert!(report.capture_complete);
                    assert_eq!(report.events_dropped, 0);
                    assert_eq!(report.failures, 0);
                    let id = |kind, index| {
                        report.node_evaluations.iter().enumerate().find_map(
                            |(position, (key, _))| {
                                (key.kind == kind && key.params == [index])
                                    .then_some(report.node_ids[position])
                            },
                        )
                    };
                    for index in 0..result.functions as u64 {
                        assert!(id(110, index).is_some(), "missing checked body {index}");
                        let proof = id(if check_only { 128 } else { 109 }, index)
                            .expect("missing body proof");
                        if check_only {
                            assert!(id(103, index).is_none());
                        } else {
                            let object = id(103, index).expect("missing object");
                            assert!(report.dependency_edges.iter().any(
                                |&(consumer, producer, output)| {
                                    producer == proof
                                        && output == 0
                                        && report.events.iter().any(|event| {
                                            event.node == Some(object)
                                                && event.task == Some(consumer)
                                        })
                                }
                            ));
                        }
                    }
                    assert_eq!(report.published_kind(103), !check_only);
                    assert_eq!(report.published_kind(105), !check_only);
                }
            }
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn rc_dyn_rejects_invalid_unused_owners_and_escaping_borrows() {
    for (name, body) in [
        (
            "missing-impl",
            "fn bad(){let _:Rc<dyn Read>=Rc::new(3u32);}",
        ),
        ("wrong-owner", "fn bad(x:Box<N>)->Rc<dyn Read>{x}"),
        (
            "moved-owner",
            "fn bad(x:Rc<N>){let _:Rc<dyn Read>=x;let _=x.read();}",
        ),
        ("escaping", "fn bad<'a>(x:Rc<dyn Read>)->&'a dyn Read{&*x}"),
        (
            "mutable",
            "fn bad(mut x:Rc<dyn Read>){let _: &mut dyn Read=&mut *x;}",
        ),
        (
            "unsized-field",
            "struct Invalid{value:dyn Read}fn bad(_x:Invalid){}",
        ),
    ] {
        let source = format!(
            "use std::rc::Rc;trait Read{{fn read(&self)->u32;}}struct N;impl Read for N{{fn read(&self)->u32{{1}}}}{body}fn main(){{}}"
        );
        let scratch = Scratch::new();
        let input = scratch.0.join("input.rs");
        fs::write(&input, source).unwrap();
        let stock = Command::new("rustc")
            .args(["--edition=2024", "-Cpanic=abort"])
            .arg(&input)
            .arg("-o")
            .arg(scratch.0.join("stock"))
            .output()
            .unwrap();
        assert!(!stock.status.success(), "{name}: stock accepted");
        for workers in [1, 8] {
            let output = scratch.0.join(format!("native-{workers}"));
            for check in [true, false] {
                let result = native(&input, (!check).then_some(output.as_path()), workers);
                assert_eq!(
                    result.status.code(),
                    Some(1),
                    "{name}/{workers}/{check}: {result:?}"
                );
                assert!(
                    !output.exists(),
                    "{name}: rejected owner published executable"
                );
            }
        }
    }
}

#[test]
fn owning_dyn_carrier_and_cleanup_authority_cannot_be_transplanted() {
    use justrust_engine::compiler::{cfg, frontend, ir, traits};
    use std::sync::Arc;
    let make = || {
        let source = "use std::rc::Rc;fn rc(x:Rc<N>)->Rc<dyn Read>{x}fn boxed(x:Box<N>)->Box<dyn Read>{x}fn other(x:Rc<N>)->Rc<dyn Other>{x}struct N;trait Read{fn read(&self)->u32;}trait Other{fn other(&self)->u32;}impl Read for N{fn read(&self)->u32{1}}impl Other for N{fn other(&self)->u32{2}}fn main(){}";
        let program = native_prelude::parse(source).unwrap();
        let budget = ir::PlanBudget::default();
        let plan = frontend::PlanCtx::from_program(&program, &budget).unwrap();
        (0..3)
            .map(|i| {
                let declaration = plan.function_decl(i).unwrap();
                let facts = plan.callable_facts(declaration).unwrap();
                let body =
                    frontend::type_callable_with_plan(&program, declaration, &plan, facts.budget())
                        .unwrap();
                (body, facts)
            })
            .collect::<Vec<_>>()
    };
    let bodies = make();
    let (checked, facts) = &bodies[0];
    let context = ir::FactsCtx::Validated(facts);
    checked.validate_with_facts(context).unwrap();
    let wrapper = checked
        .exprs
        .iter()
        .position(|e| matches!(e.kind, ir::ExprKind::RefToDyn { .. }))
        .unwrap();
    let ir::ExprKind::RefToDyn { reference } = checked.exprs[wrapper].kind else {
        unreachable!()
    };
    let owner = checked.exprs[wrapper].ty.clone();
    assert_eq!(
        owner.owned_pointer_object().unwrap().0,
        ir::OwnedPointerKind::Rc
    );
    let traits::NormalDropPlan::Destructor(drop) = facts.normal_drop_plan(&owner).unwrap() else {
        panic!("Rc dyn cleanup")
    };
    assert!(matches!(drop.glue(), traits::DropGlue::RcObject));
    drop.validate(&owner, facts).unwrap();
    assert!(drop.validate(&bodies[1].0.result, facts).is_err());
    assert!(drop.validate(&bodies[2].0.result, facts).is_err());
    let foreign = make();
    assert!(drop.validate(&owner, &foreign[0].1).is_err());
    assert!(
        checked
            .validate_with_facts(ir::FactsCtx::Validated(&foreign[0].1))
            .is_err()
    );
    for mode in 0..8 {
        let mut forged = checked.clone();
        match mode {
            0 => forged.exprs[wrapper].ty = bodies[1].0.result.clone(),
            1 => forged.exprs[wrapper].ty = bodies[2].0.result.clone(),
            2 => {
                forged.exprs[reference.0].ty =
                    bodies[1].0.locals[bodies[1].0.params[0].0].ty.clone()
            }
            3 => {
                let ir::Ty::Struct(value) = &owner else {
                    unreachable!()
                };
                let mut def = value.def().clone();
                let mut fields = def.fields.read(facts.budget()).unwrap().to_vec();
                fields[1].ty = ir::Ty::Int(ir::IntTy::Usize);
                def.fields = fields.into();
                let ty = ir::Ty::Struct(ir::StructTy::from_ref(ir::NominalRef::Strong(Arc::new(
                    def,
                ))));
                assert!(ty.owned_pointer_object().is_none());
                forged.exprs[wrapper].ty = ty;
            }
            4 => forged.exprs[wrapper].kind = ir::ExprKind::Tuple(vec![]),
            5 => {
                let ir::Ty::Struct(value) = &owner else {
                    unreachable!()
                };
                let mut def = value.def().clone();
                def.name = "crate::Rc".into();
                forged.exprs[wrapper].ty = ir::Ty::Struct(ir::StructTy::from_ref(
                    ir::NominalRef::Strong(Arc::new(def)),
                ));
            }
            6 => {
                let ir::Ty::Struct(value) = &owner else {
                    unreachable!()
                };
                let ir::Ty::Struct(other) = &bodies[1].0.result else {
                    unreachable!()
                };
                let mut def = value.def().clone();
                def.source_decl = other.def().source_decl;
                forged.exprs[wrapper].ty = ir::Ty::Struct(ir::StructTy::from_ref(
                    ir::NominalRef::Strong(Arc::new(def)),
                ));
            }
            _ => {
                let ir::Ty::Struct(value) = &owner else {
                    unreachable!()
                };
                let mut def = value.def().clone();
                let mut fields = def.fields.read(facts.budget()).unwrap().to_vec();
                fields[0].ty = ir::Ty::RawPtr {
                    kind: ir::BorrowKind::Mut,
                    pointee: Arc::new(ir::Ty::Bool),
                };
                def.fields = fields.into();
                let ty = ir::Ty::Struct(ir::StructTy::from_ref(ir::NominalRef::Strong(Arc::new(
                    def,
                ))));
                assert!(ty.owned_pointer_object().is_none());
                forged.exprs[wrapper].ty = ty;
            }
        }
        assert!(
            forged.validate_with_facts(context).is_err(),
            "IR forgery {mode}"
        );
        assert!(
            cfg::lower_template(Arc::new(forged), facts.clone(), facts.budget()).is_err(),
            "CFG forgery {mode}"
        );
    }
    // The same canonical unsize is legal at an explicit return, but its
    // independently valid Other principal is not the declared Read result.
    let mut explicit = checked.clone();
    explicit.body.tail = None;
    explicit
        .body
        .statements
        .push(ir::TypedStmt::Return(Some(ir::ExprId(wrapper))));
    explicit.validate_with_facts(context).unwrap();
    cfg::lower_template(Arc::new(explicit.clone()), facts.clone(), facts.budget()).unwrap();
    explicit.exprs[wrapper].ty = bodies[2].0.result.clone();
    assert!(explicit.validate_with_facts(context).is_err());
    assert!(cfg::lower_template(Arc::new(explicit), facts.clone(), facts.budget()).is_err());
    let mut missing_tail = checked.clone();
    missing_tail.body.tail = None;
    assert!(missing_tail.validate_with_facts(context).is_err());
    assert!(cfg::lower_template(Arc::new(missing_tail), facts.clone(), facts.budget()).is_err());
    let mut missing = checked.clone();
    missing.body.tail = None;
    missing.body.statements.push(ir::TypedStmt::Return(None));
    assert!(missing.validate_with_facts(context).is_err());
    assert!(cfg::lower_template(Arc::new(missing), facts.clone(), facts.budget()).is_err());
    let template =
        cfg::lower_template(Arc::new(checked.clone()), facts.clone(), facts.budget()).unwrap();
    for mode in 0..3 {
        let mut forged = template.function().as_ref().clone();
        let located = forged
            .blocks
            .iter_mut()
            .flat_map(|b| b.statements.iter_mut())
            .find(|s| {
                matches!(
                    s.value,
                    cfg::Statement::Assign {
                        value: cfg::Rvalue::RefToDyn { .. },
                        ..
                    }
                )
            })
            .unwrap();
        if mode == 0 {
            located.origin = None;
        } else {
            let cfg::Statement::Assign {
                value: cfg::Rvalue::RefToDyn { object, boxed, .. },
                ..
            } = &mut located.value
            else {
                unreachable!()
            };
            if mode == 1 {
                *boxed = Some(Arc::new(bodies[1].0.result.clone()));
            } else {
                *object = Arc::new(bodies[2].0.result.owned_pointer_object().unwrap().1.clone());
            }
        }
        assert!(
            forged.validate_with_facts(context).is_err(),
            "CFG origin/carrier forgery {mode}"
        );
    }
}

#[test]
fn rc_dyn_check_is_codegen_free_and_build_demands_exact_pointee_drop() {
    let source = "use std::rc::Rc;trait Read{fn read(&self)->u32;}struct N{value:u32}impl Read for N{fn read(&self)->u32{self.value}}impl Drop for N{fn drop(&mut self){println!(\"drop {}\",self.value);}}fn main(){let typed=Rc::new(N{value:7});let object:Rc<dyn Read>=typed.clone();println!(\"{}\",object.read());}";
    for workers in [1, 8] {
        for check in [true, false] {
            let result = justrust_engine::compiler::compile(source.into(), workers, check).unwrap();
            assert!(result.report.capture_complete);
            assert_eq!(result.report.failures, 0);
            assert_eq!(result.executable.is_none(), check);
            assert_eq!(result.report.published_kind(103), !check);
            assert_eq!(result.report.published_kind(105), !check);
            if check {
                assert!(
                    result
                        .report
                        .node_evaluations
                        .iter()
                        .all(|(key, _)| !matches!(key.kind, 103 | 105 | 130..=133))
                );
            }
        }
    }
}
