//! Acceptance of explicit CFG edges and direct native CFG emission.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

use ir::IntTy;
use justrust_engine::compiler::{backend, cfg, frontend, ir};
use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

static NEXT_ID: AtomicU64 = AtomicU64::new(0);
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let base = std::env::var_os("JCODE_SCRATCH_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        for _ in 0..100 {
            let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
            let path = base.join(format!("jr-cfg-{}-{epoch}-{id}", std::process::id()));
            match fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("create scratch: {error}"),
            }
        }
        panic!("could not allocate scratch");
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn new_file(path: &Path) -> fs::File {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .unwrap()
}
struct Running(Child);
impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn capture(command: &mut Command, scratch: &Scratch, seconds: u64) -> Vec<u8> {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let stdout = scratch.path(&format!("stdout-{id}"));
    let stderr = scratch.path(&format!("stderr-{id}"));
    command
        .stdin(Stdio::null())
        .stdout(new_file(&stdout))
        .stderr(new_file(&stderr));
    let mut child = Running(
        command
            .spawn()
            .unwrap_or_else(|error| panic!("spawn {command:?}: {error}")),
    );
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            break status;
        }
        assert!(
            start.elapsed() < Duration::from_secs(seconds),
            "timed out: {command:?}"
        );
        std::thread::sleep(Duration::from_millis(5));
    };
    let out = fs::read(stdout).unwrap();
    let err = fs::read(stderr).unwrap();
    assert!(
        status.success(),
        "{command:?}: {status}\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out),
        String::from_utf8_lossy(&err)
    );
    out
}
fn bounded(name: &str, run: impl FnOnce()) {
    const CHILD: &str = "JR_CFG_ACCEPTANCE_CHILD";
    if std::env::var(CHILD).as_deref() == Ok(name) {
        run();
        return;
    }
    let scratch = Scratch::new();
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", name, "--nocapture"])
        .env(CHILD, name);
    capture(&mut command, &scratch, 30);
}
fn checked_function(
    source: &str,
    name: &str,
) -> (
    ir::CheckedFunction,
    std::sync::Arc<justrust_engine::compiler::traits::ValidatedFacts>,
) {
    let program = frontend::parse(source).unwrap();
    let budget = ir::PlanBudget::default();
    let plan = frontend::PlanCtx::from_program(&program, &budget).unwrap();
    let declaration = (0..plan.callable_count())
        .map(|index| plan.callable_decl(index).unwrap())
        .find(|declaration| {
            let function = plan.callable_body(&program, *declaration).unwrap();
            function.name.anchor() == frontend::PathAnchor::Crate
                && function.name.len() == 1
                && program.names.spelling(function.name.segments()[0]).unwrap() == name
        })
        .unwrap();
    let checked = frontend::type_callable_with_plan(&program, declaration, &plan, &budget).unwrap();
    let facts = plan
        .instance_facts(&ir::CallTarget {
            declaration,
            type_args: [].into(),
        })
        .unwrap();
    (checked, facts)
}
fn lowered(source: &str, name: &str) -> cfg::Function {
    let (checked, facts) = checked_function(source, name);
    let result = cfg::lower_with_facts(&checked, &facts, facts.budget()).unwrap();
    result.validate().unwrap();
    result
}
fn edges(terminator: &cfg::Terminator) -> Vec<cfg::BlockId> {
    match terminator {
        cfg::Terminator::Goto(target) => vec![*target],
        cfg::Terminator::Switch {
            then_block,
            else_block,
            ..
        }
        | cfg::Terminator::SwitchVariant {
            then_block,
            else_block,
            ..
        } => vec![*then_block, *else_block],
        cfg::Terminator::Call { next, .. } => vec![*next],
        cfg::Terminator::Return | cfg::Terminator::Panic(_) | cfg::Terminator::Unreachable => {
            vec![]
        }
    }
}
fn reachable(function: &cfg::Function) -> HashSet<cfg::BlockId> {
    let mut seen = HashSet::new();
    let mut pending = vec![function.entry];
    while let Some(block) = pending.pop() {
        if seen.insert(block) {
            pending.extend(edges(&function.blocks[block.0].terminator.value));
        }
    }
    seen
}
fn place(local: usize) -> ir::Place {
    ir::Place {
        local: ir::LocalId(local),
        projection: vec![],
    }
}
fn located<T>(value: T) -> cfg::Located<T> {
    cfg::Located {
        value,
        origin: None,
    }
}

#[test]
fn cfg_conditional_joins_short_circuit_edges_and_origins_are_explicit() {
    bounded(
        "cfg_conditional_joins_short_circuit_edges_and_origins_are_explicit",
        || {
            let source = "fn choose(flag:bool)->i32 {let mut x=1;if flag && {x=2;true} {x=3;}else{x=4;}x} fn main(){}";
            let (checked, facts) = checked_function(source, "choose");
            let function = cfg::lower_with_facts(&checked, &facts, facts.budget()).unwrap();
            function.validate().unwrap();
            assert_eq!(function.name, checked.name);
            assert_eq!(function.params, checked.params);
            assert_eq!(function.result, checked.result);
            assert!(
                function.locals.len() > checked.locals.len(),
                "CFG must own temporary snapshots"
            );
            for (actual, expected) in function.locals.iter().zip(&checked.locals) {
                assert_eq!(actual.name, expected.name);
                assert_eq!(actual.ty, expected.ty);
            }
            let switches = function
                .blocks
                .iter()
                .filter(|block| matches!(block.terminator.value, cfg::Terminator::Switch { .. }))
                .count();
            assert!(
                switches >= 2,
                "&& and if require explicit conditional edges"
            );
            let mut origins = 0;
            for block in &function.blocks {
                for target in edges(&block.terminator.value) {
                    assert!(target.0 < function.blocks.len());
                }
                for origin in block
                    .statements
                    .iter()
                    .filter_map(|item| item.origin)
                    .chain(block.terminator.origin)
                {
                    origins += 1;
                    assert!(
                        origin.0 < checked.exprs.len(),
                        "origin must refer to checked expression"
                    );
                }
            }
            assert!(origins > 0, "lowering lost diagnostic provenance");
            let live = reachable(&function);
            assert!(live.iter().any(|block| matches!(
                function.blocks[block.0].terminator.value,
                cfg::Terminator::Return
            )));
        },
    );
}

#[test]
fn cfg_recursive_calls_use_signatures_without_checking_other_bodies() {
    bounded(
        "cfg_recursive_calls_use_signatures_without_checking_other_bodies",
        || {
            let source = "fn recurse(n:i32)->(i32,bool){if n==0 {(0,true)} else {recurse(n-1)}} fn invalid_unused(){let x:bool=1;} fn main(){}";
            let function = lowered(source, "recurse");
            let calls = function
                .blocks
                .iter()
                .filter_map(|block| match &block.terminator.value {
                    cfg::Terminator::Call {
                        name,
                        signature,
                        args,
                        next,
                        ..
                    } => Some((name, signature, args, next)),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(calls.len(), 1);
            let (name, signature, args, next) = calls[0];
            // Diagnostic paths are canonical now. Recursion must still name
            // this exact lowered function, not a separately printed alias.
            assert_eq!(name, &function.name);
            assert_eq!(signature.params, [ir::Ty::Int(IntTy::I32)]);
            assert_eq!(signature.result, function.result);
            assert_eq!(args.len(), 1);
            assert!(next.0 < function.blocks.len());
        },
    );
}

#[test]
fn cfg_loop_transfers_have_no_implicit_fallthrough_return() {
    bounded(
        "cfg_loop_transfers_have_no_implicit_fallthrough_return",
        || {
            let infinite = lowered("fn main(){loop {continue;}}", "main");
            let live = reachable(&infinite);
            assert!(!live.is_empty());
            assert!(live.iter().all(|block| !matches!(
                infinite.blocks[block.0].terminator.value,
                cfg::Terminator::Return
            )));
            assert!(
                live.iter()
                    .any(|block| !edges(&infinite.blocks[block.0].terminator.value).is_empty())
            );
            for source in [
                "fn value()->i32{loop {break 7;}} fn main(){}",
                "fn value()->i32{loop {return 7;}} fn main(){}",
                "fn value()->i32{let mut n=0;loop{n=n+1;if n<2{continue;}break n;}} fn main(){}",
            ] {
                let function = lowered(source, "value");
                assert!(reachable(&function).iter().any(|block| matches!(
                    function.blocks[block.0].terminator.value,
                    cfg::Terminator::Return
                )));
            }
        },
    );
}

#[test]
fn cfg_validator_rejects_malformed_edges_locals_places_and_call_arity() {
    bounded(
        "cfg_validator_rejects_malformed_edges_locals_places_and_call_arity",
        || {
            let original = lowered("fn main(){}", "main");
            let mut function = original.clone();
            function.entry = cfg::BlockId(function.blocks.len());
            assert!(function.validate().is_err(), "invalid entry accepted");
            let mut function = original.clone();
            function.blocks.clear();
            assert!(function.validate().is_err(), "empty graph accepted");
            let mut function = original.clone();
            function.blocks[function.entry.0].terminator =
                located(cfg::Terminator::Goto(cfg::BlockId(function.blocks.len())));
            assert!(function.validate().is_err(), "invalid goto accepted");
            let mut function = original.clone();
            function.return_local = ir::LocalId(function.locals.len());
            assert!(
                function.validate().is_err(),
                "invalid return local accepted"
            );
            let mut function = original.clone();
            function.blocks[function.entry.0]
                .statements
                .push(located(cfg::Statement::Assign {
                    destination: place(function.locals.len()),
                    value: cfg::Rvalue::Use(cfg::Operand::Constant(cfg::Constant::Unit)),
                }));
            assert!(function.validate().is_err(), "invalid destination accepted");
            let mut function = original.clone();
            function.blocks[function.entry.0].statements.push(located(
                cfg::Statement::StorageLive(ir::LocalId(function.locals.len())),
            ));
            assert!(
                function.validate().is_err(),
                "invalid storage local accepted"
            );
            let mut function = original.clone();
            function.blocks[function.entry.0].terminator = located(cfg::Terminator::Switch {
                condition: cfg::Operand::Constant(cfg::Constant::Bool(true)),
                then_block: function.entry,
                else_block: cfg::BlockId(function.blocks.len()),
            });
            assert!(function.validate().is_err(), "invalid switch edge accepted");
            let mut function = original.clone();
            function.blocks[function.entry.0]
                .statements
                .push(located(cfg::Statement::Assign {
                    destination: place(function.return_local.0),
                    value: cfg::Rvalue::Use(cfg::Operand::Copy(ir::Place {
                        local: function.return_local,
                        projection: vec![ir::Projection::Field(0)],
                    })),
                }));
            assert!(function.validate().is_err(), "invalid projection accepted");
            let mut function = lowered("fn callee(n:i32){} fn main(){callee(1);}", "main");
            let call = function
                .blocks
                .iter_mut()
                .find_map(|block| match &mut block.terminator.value {
                    cfg::Terminator::Call { args, .. } => Some(args),
                    _ => None,
                })
                .unwrap();
            call.clear();
            assert!(function.validate().is_err(), "invalid call arity accepted");
        },
    );
}

const NATIVE: &str = r#"
fn encode(value:(i32,i32),last:i32)->i32 { value.0*100+value.1*10+last }
fn scalar_abrupt()->i32 {return {return 3;};}
fn tuple_abrupt()->(i32,bool) {let pending=(1,{return (7,true);});(0,false)}
fn recurse(n:i32)->(i32,bool) { if n==0 {(1,true)} else {let before=recurse(n-1);(before.0+1,before.1)} }
fn control(flag:bool)->i32 {
    let mut n=0;
    loop {n=n+1;if flag && n==2 {return n;}if n<3 {continue;}break n;}
}
fn main() {
    let mut order=0;
    let skip_left=false && {order=99;true};
    let skip_right=true || {order=99;false};
    let run=true && {order=order*10+1;true};
    let mut value=(1,2);
    let saved=encode(value,{order=order*10+2;value.0=9;3});
    let selected=if saved==123 {(7,true)} else {(0,false)};
    let recursive=recurse(5);
    let abrupt=tuple_abrupt();
    let mut attempts=0;
    let aggregate=loop {attempts=attempts+1;encode((1,2),{if attempts<2 {continue;}break (8,true);0});};
    let mut w=0;
    while {let condition_snapshot=w<3;condition_snapshot} {
        let body_snapshot=w;
        w=w+1;
        if body_snapshot==0 {continue;}
        break;
    }
    if !skip_left && skip_right && run && order==12 && saved==123 && value.0==9 && selected.0==7 && selected.1 && recursive.0==6 && recursive.1 && control(true)==2 && control(false)==3 && aggregate.0==8 && aggregate.1 && attempts==2 && scalar_abrupt()==3 && abrupt.0==7 && abrupt.1 && w==2 {
        println!("cfg native ok");
    }
}
"#;

#[test]
fn cfg_return_snapshots_and_abrupt_edges_clean_only_exited_scopes() {
    bounded(
        "cfg_return_snapshots_and_abrupt_edges_clean_only_exited_scopes",
        || {
            let (checked, facts) = checked_function(
                "fn pick()->(i32,bool){let outer=(3,true);{let inner=outer;return inner;}} fn main(){}",
                "pick",
            );
            let function = cfg::lower_with_facts(&checked, &facts, facts.budget()).unwrap();
            function.validate().unwrap();
            for id in reachable(&function) {
                let block = &function.blocks[id.0];
                if !matches!(block.terminator.value, cfg::Terminator::Return) {
                    continue;
                }
                let copied = block.statements.iter().position(|statement| matches!(&statement.value, cfg::Statement::Assign { destination, .. } if destination.local == function.return_local && destination.projection.is_empty())).expect("return value was not snapshotted");
                for (index, statement) in block.statements.iter().enumerate() {
                    if let cfg::Statement::StorageDead(local) = statement.value {
                        assert_ne!(
                            local, function.return_local,
                            "return snapshot cannot die before Return"
                        );
                        if local.0 < checked.locals.len() {
                            assert!(copied < index, "source local died before return snapshot");
                        }
                    }
                }
            }
            let (checked, facts) = checked_function(
                "fn main(){let mut count=0;loop{let local=(count,true);{let nested=local;count=count+1;if count<2{continue;}break;}}}",
                "main",
            );
            let function = cfg::lower_with_facts(&checked, &facts, facts.budget()).unwrap();
            function.validate().unwrap();
            let local_id = |name: &str| {
                ir::LocalId(
                    checked
                        .locals
                        .iter()
                        .position(|local| local.name == name)
                        .unwrap(),
                )
            };
            let count = local_id("count");
            let local = local_id("local");
            let nested = local_id("nested");
            let mut transfers = 0;
            for block in &function.blocks {
                let Some(origin) = block.terminator.origin else {
                    continue;
                };
                if !matches!(
                    checked.exprs[origin.0].kind,
                    ir::ExprKind::Break { .. } | ir::ExprKind::Continue { .. }
                ) {
                    continue;
                }
                transfers += 1;
                assert!(matches!(block.terminator.value, cfg::Terminator::Goto(_)));
                let dead = block
                    .statements
                    .iter()
                    .filter_map(|statement| match statement.value {
                        cfg::Statement::StorageDead(local) => Some(local),
                        _ => None,
                    })
                    .collect::<HashSet<_>>();
                assert!(
                    dead.contains(&local) && dead.contains(&nested),
                    "exited block locals must die on abrupt edge"
                );
                assert!(!dead.contains(&count), "outer loop state must remain live");
            }
            assert_eq!(transfers, 2, "expected distinct break and continue edges");
        },
    );
}

#[test]
fn cfg_native_emission_preserves_order_snapshots_recursion_and_control_edges() {
    bounded(
        "cfg_native_emission_preserves_order_snapshots_recursion_and_control_edges",
        || {
            use std::os::unix::fs::PermissionsExt;
            let program = frontend::parse(NATIVE).unwrap();
            let budget = ir::PlanBudget::default();
            let plan = frontend::PlanCtx::from_program(&program, &budget).unwrap();
            let functions = (0..plan.callable_count())
                .filter(|index| {
                    matches!(
                        plan.callable_provider(plan.callable_decl(*index).unwrap())
                            .unwrap(),
                        frontend::CallableProvider::Source { .. }
                    )
                })
                .map(|index| {
                    let declaration = plan.callable_decl(index).unwrap();
                    let checked =
                        frontend::type_callable_with_plan(&program, declaration, &plan, &budget)
                            .unwrap();
                    let facts = plan
                        .instance_facts(&ir::CallTarget {
                            declaration,
                            type_args: [].into(),
                        })
                        .unwrap();
                    let lowered = cfg::lower_with_facts(&checked, &facts, facts.budget()).unwrap();
                    lowered.validate().unwrap();
                    lowered
                })
                .collect::<Vec<_>>();
            let cfg_blocks: usize = functions.iter().map(|function| function.blocks.len()).sum();
            let cfg_locals: usize = functions.iter().map(|function| function.locals.len()).sum();
            let cfg_edges: usize = functions
                .iter()
                .flat_map(|function| &function.blocks)
                .map(|block| edges(&block.terminator.value).len())
                .sum();
            assert!(cfg_blocks > 0 && cfg_edges > 0 && cfg_locals > 0);
            let direct = justrust_engine::compiler::compile(NATIVE.to_owned(), 1, false)
                .unwrap()
                .executable
                .unwrap();
            let compatibility = backend::emit(&program).unwrap();
            let scratch = Scratch::new();
            for (name, bytes) in [("cfg", direct), ("compatibility", compatibility)] {
                assert!(bytes.len() >= 64);
                assert_eq!(&bytes[..6], b"\x7fELF\x02\x01");
                let path = scratch.path(name);
                new_file(&path).write_all(&bytes).unwrap();
                fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
                assert_eq!(
                    capture(&mut Command::new(path), &scratch, 5),
                    b"cfg native ok\n"
                );
            }
            for workers in [1, 8] {
                for check in [true, false] {
                    let result =
                        justrust_engine::compiler::compile(NATIVE.to_owned(), workers, check)
                            .unwrap();
                    assert!(result.report.capture_complete);
                    assert_eq!(result.report.failures, 0);
                    assert_eq!(result.report.events_dropped, 0);
                    assert_eq!(result.executable.is_none(), check);
                    assert_eq!(result.cfg_blocks, cfg_blocks);
                    assert_eq!(result.cfg_edges, cfg_edges);
                    assert_eq!(result.cfg_locals, cfg_locals);
                    assert_eq!(result.control_flow.len(), functions.len());
                    assert_eq!(result.analyses.len(), result.functions);
                    assert!(result.analyses.iter().all(|analysis| analysis.passed()));
                    let analyses = result
                        .report
                        .node_evaluations
                        .iter()
                        .filter(|(key, _)| key.kind == 109)
                        .collect::<Vec<_>>();
                    assert_eq!(analyses.len(), result.functions);
                    for index in 0..result.functions {
                        let key = justrust_engine::Key::new(109, vec![index as u64]);
                        assert_eq!(
                            analyses
                                .iter()
                                .filter(|(seen, count)| *seen == key && *count == 1)
                                .count(),
                            1,
                            "analysis must run exactly once for {key:?}"
                        );
                    }
                    assert_eq!(
                        result
                            .control_flow
                            .iter()
                            .map(|function| function.blocks.len())
                            .sum::<usize>(),
                        result.cfg_blocks
                    );
                    assert_eq!(
                        result
                            .control_flow
                            .iter()
                            .map(|function| function.locals.len())
                            .sum::<usize>(),
                        result.cfg_locals
                    );
                    assert_eq!(
                        result
                            .control_flow
                            .iter()
                            .flat_map(|function| &function.blocks)
                            .map(|block| edges(&block.terminator.value).len())
                            .sum::<usize>(),
                        result.cfg_edges
                    );
                    for (retained, independent) in result.control_flow.iter().zip(&functions) {
                        retained.validate().unwrap();
                        assert_eq!(retained.name, independent.name);
                    }
                    if let Some(bytes) = result.executable {
                        let path = scratch.path(&format!("graph-{workers}"));
                        new_file(&path).write_all(&bytes).unwrap();
                        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
                        assert_eq!(
                            capture(&mut Command::new(path), &scratch, 5),
                            b"cfg native ok\n"
                        );
                    }
                }
            }
        },
    );
}

#[test]
fn cfg_cli_graph_trace_is_explicit_and_normal_output_remains_json() {
    bounded(
        "cfg_cli_graph_trace_is_explicit_and_normal_output_remains_json",
        || {
            for workers in [1, 8] {
                for check in [true, false] {
                    for tracing in [false, true] {
                        let scratch = Scratch::new();
                        let input = scratch.path("input.rs");
                        new_file(&input)
                            .write_all(b"fn helper(x:i32)->i32{x+1} fn main(){helper(1);}")
                            .unwrap();
                        let output = scratch.path("program");
                        let trace = scratch.path("trace");
                        let mut command = Command::new(env!("CARGO_BIN_EXE_jr-rust"));
                        command
                            .arg(&input)
                            .args(["--workers", &workers.to_string()]);
                        if check {
                            command.arg("--check");
                        } else {
                            command.arg("-o").arg(&output);
                        }
                        if tracing {
                            command.arg("--trace").arg(&trace);
                        }
                        let stdout =
                            String::from_utf8(capture(&mut command, &scratch, 20)).unwrap();
                        assert!(
                            stdout.starts_with('{') && stdout.ends_with("}\n"),
                            "CLI stdout must remain one JSON object: {stdout}"
                        );
                        assert_eq!(stdout.lines().count(), 1);
                        for field in [
                            "\"cfg_blocks\":",
                            "\"cfg_edges\":",
                            "\"cfg_locals\":",
                            "\"analysis_worklist_pops\":",
                            "\"analysis_move_paths\":",
                            "\"analysis_moves\":",
                        ] {
                            assert!(
                                stdout.contains(field),
                                "missing CFG telemetry {field}: {stdout}"
                            );
                        }
                        for field in ["analysis_worklist_pops", "analysis_move_paths"] {
                            let prefix = format!("\"{field}\":");
                            let value = stdout
                                .split_once(&prefix)
                                .unwrap()
                                .1
                                .split([',', '}'])
                                .next()
                                .unwrap()
                                .parse::<usize>()
                                .unwrap();
                            assert!(value > 0, "missing analysis work for {field}: {stdout}");
                        }
                        for full_graph in [
                            "control-flow graphs:",
                            "initialization/move analyses:",
                            "BlockId(",
                            "origin:",
                        ] {
                            assert!(
                                !stdout.contains(full_graph),
                                "implicit graph dump on stdout"
                            );
                        }
                        assert_eq!(output.exists(), !check);
                        if tracing {
                            let text = fs::read_to_string(&trace).unwrap();
                            for required in [
                                "control-flow graphs:",
                                "initialization/move analyses:",
                                "helper",
                                "main",
                                "BlockId(",
                                "origin:",
                            ] {
                                assert!(
                                    text.contains(required),
                                    "missing {required} from explicit trace: {text}"
                                );
                            }
                        } else {
                            assert!(!trace.exists());
                        }
                    }
                }
            }
        },
    );
}
