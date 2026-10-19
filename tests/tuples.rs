//! Native tuple acceptance. Stock Rust is used only as an external development oracle.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
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
            let path = base.join(format!("jr-tuples-{}-{epoch}-{id}", std::process::id()));
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
    fn source(&self, source: &str) -> PathBuf {
        let path = self.path("input.rs");
        new_file(&path).write_all(source.as_bytes()).unwrap();
        path
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn new_file(path: &Path) -> fs::File {
    OpenOptions::new()
        .create_new(true)
        .write(true)
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
#[derive(Debug)]
struct Captured {
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}
fn capture(command: &mut Command, scratch: &Scratch, seconds: u64) -> Captured {
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
    Captured {
        status,
        stdout: fs::read(stdout).unwrap(),
        stderr: fs::read(stderr).unwrap(),
    }
}
fn compiler(input: &Path, output: &Path, workers: usize, check: bool) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_jr-rust"));
    command.arg(input).args(["--workers", &workers.to_string()]);
    if check {
        command.arg("--check");
    } else {
        command.arg("-o").arg(output);
    }
    command
}
fn runs(source: &str, expected: &str) {
    for workers in [1, 8] {
        let scratch = Scratch::new();
        let input = scratch.source(source);
        let output = scratch.path("program");
        for check in [true, false] {
            let result = capture(&mut compiler(&input, &output, workers, check), &scratch, 20);
            assert!(
                result.status.success(),
                "workers={workers}, check={check}: {result:?}\n{}\n{source}",
                String::from_utf8_lossy(&result.stderr)
            );
            if check {
                assert!(!output.exists(), "check published executable");
            }
        }
        let bytes = fs::read(&output).unwrap();
        assert!(bytes.len() >= 64, "truncated ELF");
        assert_eq!(&bytes[..6], b"\x7fELF\x02\x01");
        let result = capture(&mut Command::new(output), &scratch, 5);
        assert!(
            result.status.success(),
            "runtime failure: {result:?}\n{source}"
        );
        assert_eq!(
            result.stdout,
            expected.as_bytes(),
            "workers={workers}\n{source}"
        );
        assert!(
            result.stderr.is_empty(),
            "unexpected runtime stderr: {result:?}"
        );
    }
}
fn rejects(label: &str, source: &str) {
    for workers in [1, 8] {
        let scratch = Scratch::new();
        let input = scratch.source(source);
        let output = scratch.path("must-not-exist");
        for check in [true, false] {
            let result = capture(&mut compiler(&input, &output, workers, check), &scratch, 20);
            assert!(
                !result.status.success(),
                "{label}: accepted workers={workers}, check={check}\n{source}"
            );
            assert!(
                result.status.code().is_some(),
                "{label}: compiler terminated by signal: {result:?}"
            );
            assert!(!result.stderr.is_empty(), "{label}: no diagnostic");
            assert!(
                !String::from_utf8_lossy(&result.stderr).contains("panicked at"),
                "{label}: compiler panic: {result:?}"
            );
            assert!(!output.exists(), "{label}: published executable");
        }
    }
}

#[test]
fn nested_tuple_literals_types_projections_and_mutable_writes() {
    runs(
        r#"
fn main() {
    let mut value:((i32,bool),((),(i32,)))=((3,true),((),(5,)));
    (value).0.0=7;
    value.1.1.0=value.0.0+2;
    let before=value.1.1.0;
    value.0=(11,false);
    let singleton=(17,);
    if before==9 && value.0.0==11 && !value.0.1 && singleton.0==17 {
        println!("tuple projections ok");
    }
}
"#,
        "tuple projections ok\n",
    );
}

#[test]
fn tuple_whole_values_flow_through_local_assignment_if_block_call_and_return() {
    runs(
        r#"
fn make(flag:bool)->((i32,bool),(i32,)) {
    if flag { ((3,true),(5,)) } else { ((7,false),(11,)) }
}
fn relay(value:((i32,bool),(i32,)))->((i32,bool),(i32,)) { return value; }
fn main() {
    let mut value=make(true);
    let copy=value;
    value=relay({let local=make(false); local});
    let selected=if copy.0.1 {copy} else {value};
    let call_projection=make(true).0.0;
    let block_projection={copy}.1.0;
    let if_projection=(if false {copy} else {value}).0.0;
    if value.0.0==7 && value.1.0==11 && selected.0.0==3 && selected.1.0==5 && call_projection==3 && block_projection==5 && if_projection==7 {
        println!("tuple flow ok");
    }
}
"#,
        "tuple flow ok\n",
    );
}

#[test]
fn tuple_arguments_projections_and_literal_elements_are_value_snapshots() {
    runs(
        r#"
fn encode(value:(i32,i32),last:i32)->i32 { value.0*100+value.1*10+last }
fn main() {
    let mut source=(1,2);
    let first=encode(source,{source.0=9; source.1=8; 3});
    let copy=source;
    let scalar=source.0+{source.0=7; 1};
    let literal=(source.0,{source.0=6; source.1},source.0);
    let mut nested=((4,5),6);
    let projected=encode(nested.0,{nested.0.0=9; nested.0.1=8; 7});
    let saved=loop {break source;};
    source.1=3;
    let mut swap=((1,2),(3,4));
    swap=(swap.1,swap.0);
    swap.1.0=9;
    if first==123 && copy.0==9 && copy.1==8 && scalar==10 && source.0==6 && literal.0==7 && literal.1==8 && literal.2==6 && projected==457 && nested.0.0==9 && saved.1==8 && source.1==3 && swap.0.0==3 && swap.0.1==4 && swap.1.1==2 {
        println!("tuple snapshots ok");
    }
}
"#,
        "tuple snapshots ok\n",
    );
}

#[test]
fn tuple_callee_copies_and_recursive_aggregate_returns_have_independent_storage() {
    runs(
        r#"
fn mutate(value:(i32,i32))->(i32,i32) {
    let mut local=value;
    local.0=99;
    local
}
fn fibonacci(n:i32)->(i32,i32) {
    if n==0 {(0,1)} else {let previous=fibonacci(n-1); (previous.1,previous.0+previous.1)}
}
fn tree(n:i32)->(i32,i32) {
    if n==0 {(1,1)} else {let first=tree(n-1); let second=tree(n-1); (first.0+second.0,first.1+second.1+1)}
}
fn main() {
    let original=(3,4);
    let changed=mutate(original);
    let fib=fibonacci(8);
    let branches=tree(3);
    if original.0==3 && original.1==4 && changed.0==99 && changed.1==4 && fib.0==21 && fib.1==34 && branches.0==8 && branches.1==15 {
        println!("tuple recursion ok");
    }
}
"#,
        "tuple recursion ok\n",
    );
}

#[test]
fn zero_sized_nested_tuple_arguments_returns_and_side_effect_order() {
    runs(
        r#"
fn empty(value:((),((),)))->((),((),)) { value }
fn consume(value:(),other:((),((),)))->i32 { 7 }
fn main() {
    let mut order=0;
    let value=({order=order*10+1; ()},({order=order*10+2; ()},));
    let returned=empty({order=order*10+3;value});
    let answer=consume(returned.0,returned);
    let after=(returned,{order=order*10+4; 9});
    if order==1234 && answer==7 && after.1==9 { println!("tuple zero sized ok"); }
}
"#,
        "tuple zero sized ok\n",
    );
}

#[test]
fn aggregate_early_returns_discard_pending_arguments_fields_and_operands() {
    runs(
        r#"
fn consume(value:(i32,i32),other:i32)->i32 { value.0+value.1+other }
fn argument_escape()->(i32,bool) {
    consume((1,2),{return (7,true); 0});
    (0,false)
}
fn field_escape()->(i32,bool) {
    let ignored=(17,{return (9,false); 0});
    (0,true)
}
fn operand_escape()->(i32,bool) {
    10+{return (11,true); 0};
    (0,false)
}
fn main() {
    let first=argument_escape();
    let second=field_escape();
    let third=operand_escape();
    if first.0==7 && first.1 && second.0==9 && !second.1 && third.0==11 && third.1 {
        println!("tuple early return ok");
    }
}
"#,
        "tuple early return ok\n",
    );
}

#[test]
fn aggregate_loop_breaks_restore_temporaries_and_preserve_outer_values() {
    runs(
        r#"
fn consume(value:(i32,i32),other:i32)->i32 { value.0+value.1+other }
fn combine(first:(i32,bool),second:(i32,bool))->i32 { first.0*10+second.0 }
fn main() {
    let mut attempts=0;
    let first=loop {
        attempts=attempts+1;
        consume((1,2),{if attempts<3 {continue;} break (7,false); 0});
    };
    let nested=loop {let inner=loop {break (3,(true,5));}; break inner;};
    let result=combine((4,true),loop {break (6,false);});
    if first.0==7 && !first.1 && attempts==3 && nested.0==3 && nested.1.0 && nested.1.1==5 && result==46 {
        println!("tuple loop break ok");
    }
}
"#,
        "tuple loop break ok\n",
    );
}

#[test]
fn tuple_type_arity_projection_and_assignment_errors_reject() {
    // Nonlocal place mutations below can be legal stock Rust, but this
    // milestone deliberately accepts assignment only through local roots.
    for (label, source) in [
        (
            "tuple local type mismatch",
            "fn main(){let x:(i32,bool)=(1,2);}",
        ),
        ("tuple arity mismatch", "fn main(){let x:(i32,)=(1,2);}"),
        (
            "tuple call type mismatch",
            "fn take(x:(i32,bool)){} fn main(){take((true,1));}",
        ),
        (
            "tuple call arity mismatch",
            "fn take(x:(i32,i32)){} fn main(){take((1,));}",
        ),
        (
            "tuple return type mismatch",
            "fn f()->(i32,bool){(1,2)} fn main(){}",
        ),
        (
            "tuple if branch mismatch",
            "fn main(){let x=if true {(1,true)}else{(1,2)};}",
        ),
        (
            "tuple break mismatch",
            "fn main(){let x=loop{if true {break (1,true);}break (2,3);};}",
        ),
        (
            "tuple whole assignment mismatch",
            "fn main(){let mut x=(1,true);x=(2,3);}",
        ),
        (
            "tuple projection assignment mismatch",
            "fn main(){let mut x=(1,true);x.0=false;}",
        ),
        (
            "tuple out of range read",
            "fn main(){let x=(1,2);let y=x.2;}",
        ),
        (
            "tuple nested out of range read",
            "fn main(){let x=((1,),2);let y=x.0.1;}",
        ),
        (
            "tuple out of range write",
            "fn main(){let mut x=(1,2);x.2=3;}",
        ),
        ("unit projection", "fn main(){let x=();let y=x.0;}"),
        ("scalar projection", "fn main(){let x=1;let y=x.0;}"),
        ("immutable tuple field", "fn main(){let x=(1,2);x.0=3;}"),
        (
            "immutable nested tuple field",
            "fn main(){let x=((1,2),3);x.0.1=4;}",
        ),
        ("immutable tuple whole", "fn main(){let x=(1,2);x=(3,4);}"),
        (
            "immutable tuple parameter",
            "fn f(x:(i32,i32)){x.0=3;} fn main(){}",
        ),
        (
            "nonlocal call projection assignment",
            "fn f()->(i32,i32){(1,2)} fn main(){f().0=3;}",
        ),
        ("nonlocal tuple literal assignment", "fn main(){(1,2).0=3;}"),
        (
            "nonlocal block projection assignment",
            "fn main(){let mut x=(1,2);{x}.0=3;}",
        ),
        (
            "unused invalid tuple body",
            "fn unused(){let x=(1,2);let y=x.7;} fn main(){}",
        ),
    ] {
        rejects(label, source);
    }
}

#[test]
fn named_struct_declaration_is_no_longer_a_tuple_milestone_boundary() {
    for workers in [1, 8] {
        for check in [true, false] {
            let compiled = justrust_engine::compiler::compile(
                "struct Point {x:i32} fn main(){}".to_owned(),
                workers,
                check,
            )
            .unwrap();
            assert_eq!(compiled.structs, 1);
            assert_eq!(compiled.executable.is_none(), check);
            assert!(compiled.analyses.iter().all(|analysis| analysis.passed()));
        }
    }
}

#[test]
fn tuple_destructuring_is_no_longer_a_milestone_boundary() {
    for workers in [1, 8] {
        for check in [true, false] {
            let compiled = justrust_engine::compiler::compile(
                "fn main(){let (a,b)=(1,2);}".to_owned(),
                workers,
                check,
            )
            .unwrap();
            assert_eq!(compiled.executable.is_none(), check);
            assert!(compiled.analyses.iter().all(|analysis| analysis.passed()));
        }
    }
}

#[test]
fn tuple_reference_is_no_longer_a_milestone_boundary() {
    for workers in [1, 8] {
        for check in [true, false] {
            let compiled = justrust_engine::compiler::compile(
                "fn main(){let x=(1,2);let y=&x;}".to_owned(),
                workers,
                check,
            )
            .unwrap();
            assert_eq!(compiled.executable.is_none(), check);
            assert!(compiled.analyses.iter().all(|analysis| analysis.passed()));
        }
    }
}

#[test]
fn tuple_milestone_unsupported_features_reject_explicitly() {
    // Some are valid stock Rust, but outside this native tuple milestone.
    for (label, source) in [
        ("tuple equality", "fn main(){let x=(1,2)==(1,2);}"),
        ("tuple ordering", "fn main(){let x=(1,2)<(2,3);}"),
        ("tuple arithmetic", "fn main(){let x=(1,2)+(3,4);}"),
        (
            "named field projection",
            "fn main(){let x=(1,2);let y=x.field;}",
        ),
    ] {
        rejects(label, source);
    }
}

#[test]
fn tuple_checked_bodies_are_shared_once_and_check_does_not_demand_codegen() {
    const CHILD: &str = "JR_TUPLE_GRAPH_ACCEPTANCE_CHILD";
    if std::env::var_os(CHILD).is_some() {
        use justrust_engine::{Key, compiler};
        let source = "fn leaf(n:i32)->(i32,bool){(n,n>0)} fn relay(n:i32)->((i32,bool),i32){(leaf(n),n+1)} fn main(){let value=relay(3);if value.0.1 && value.1==4 {println!(\"graph tuple ok\");}}";
        for workers in [1, 8] {
            for check in [true, false] {
                let result = compiler::compile(source.to_owned(), workers, check).unwrap();
                assert_eq!(result.functions, 3);
                assert_eq!(result.analyses.len(), result.functions);
                assert!(result.analyses.iter().all(|analysis| analysis.passed()));
                assert_eq!(result.executable.is_none(), check);
                let report = result.report;
                assert!(report.capture_complete);
                assert_eq!(report.events_dropped, 0);
                assert_eq!(report.failures, 0);
                let mut expected = vec![
                    // Text now uses the same configured SOURCE transport as files.
                    Key::new(107, vec![0]),
                    Key::new(100, vec![]),
                    Key::new(106, vec![]),
                    Key::new(101, vec![]),
                    Key::new(104, vec![]),
                    Key::new(113, vec![]),
                    Key::new(115, vec![]),
                ];
                for index in 0..3 {
                    expected.push(Key::new(110, vec![index]));
                    expected.push(Key::new(102, vec![index]));
                    expected.push(Key::new(108, vec![index]));
                    expected.push(Key::new(109, vec![index]));
                    if !check {
                        expected.push(Key::new(103, vec![index]));
                    }
                }
                if !check {
                    expected.push(Key::new(105, vec![]));
                }
                // Available catalog recipes add real universal proof nodes, but
                // never eager concrete roots. Keep every historical source key.
                let program = compiler::frontend::parse(source).unwrap();
                let plan = compiler::frontend::PlanCtx::from_program(
                    &program,
                    &compiler::ir::PlanBudget::default(),
                )
                .unwrap();
                for index in 0..plan.callable_count() {
                    let declaration = plan.callable_decl(index).unwrap();
                    match plan.callable_provider(declaration).unwrap() {
                        compiler::frontend::CallableProvider::Source { .. } => {
                            assert!(index < result.functions);
                        }
                        compiler::frontend::CallableProvider::Recipe { .. } => {
                            assert!(index >= result.functions);
                            for kind in [110, 111, 112] {
                                expected.push(Key::new(kind, vec![index as u64]));
                            }
                        }
                        compiler::frontend::CallableProvider::UnavailableProvided { .. } => {
                            panic!("unavailable provider entered body enumeration")
                        }
                    }
                }
                // Effect publication joins every original body and required universal
                // proof before the instance plan can publish any concrete work.
                let discovered = |key: &Key| {
                    let spelling = format!("{key:?}");
                    report
                        .events
                        .iter()
                        .find(|event| event.kind == "discovered" && event.detail == spelling)
                        .expect("required graph node was not discovered")
                };
                let published = |node| {
                    report
                        .events
                        .iter()
                        .find(|event| event.kind == "publish" && event.node == Some(node))
                        .expect("required graph node did not publish")
                        .at_ns
                };
                let effects = published(discovered(&Key::new(115, vec![])).node.unwrap());
                for (key, _) in &report.node_evaluations {
                    let event = discovered(key);
                    if matches!(key.kind, 110 | 112) {
                        assert!(
                            published(event.node.unwrap()) <= effects,
                            "effect publication preceded mandatory body proof: {key:?}"
                        );
                    }
                }
                if check {
                    assert!(
                        !report
                            .node_evaluations
                            .iter()
                            .any(|(key, _)| key.kind == 103)
                    );
                } else {
                    // Each object follows its own instance proof (no global barrier).
                    for (key, _) in &report.node_evaluations {
                        if key.kind == 103 {
                            let proof = Key::new(109, key.params.clone());
                            let proved = published(discovered(&proof).node.unwrap());
                            assert!(published(discovered(key).node.unwrap()) >= proved);
                        }
                    }
                }
                assert_eq!(report.node_evaluations.len(), expected.len());
                for key in expected {
                    let entries: Vec<_> = report
                        .node_evaluations
                        .iter()
                        .filter(|(seen, _)| *seen == key)
                        .collect();
                    assert_eq!(entries.len(), 1, "missing or duplicated {key:?}");
                    assert_eq!(entries[0].1, 1, "shared tuple operation repeated {key:?}");
                }
            }
        }
        return;
    }
    let scratch = Scratch::new();
    let mut child = Command::new(std::env::current_exe().unwrap());
    child
        .args([
            "--exact",
            "tuple_checked_bodies_are_shared_once_and_check_does_not_demand_codegen",
            "--nocapture",
        ])
        .env(CHILD, "1");
    let result = capture(&mut child, &scratch, 20);
    assert!(
        result.status.success(),
        "tuple graph child failed: {result:?}\n{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}
