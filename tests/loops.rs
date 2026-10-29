//! Bounded native CLI acceptance of loops, break and continue.
//! Range fixtures also execute stock rustc as an external oracle.
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
            let path = base.join(format!("jr-loops-{}-{epoch}-{id}", std::process::id()));
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
#[derive(Debug)]
struct Captured {
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}
impl Captured {
    fn success(&self, context: &str) {
        assert!(
            self.status.success(),
            "{context}: {self:?}\n{}",
            String::from_utf8_lossy(&self.stderr)
        );
    }
}
fn capture(command: &mut Command, scratch: &Scratch, timeout: Duration) -> Captured {
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
        assert!(start.elapsed() < timeout, "timed out: {command:?}");
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
            let result = capture(
                &mut compiler(&input, &output, workers, check),
                &scratch,
                Duration::from_secs(20),
            );
            result.success(source);
            if check {
                assert!(!output.exists(), "check published executable");
            }
        }
        let bytes = fs::read(&output).unwrap();
        assert!(bytes.len() >= 64, "truncated ELF");
        assert_eq!(&bytes[..6], b"\x7fELF\x02\x01");
        let result = capture(&mut Command::new(output), &scratch, Duration::from_secs(5));
        result.success(source);
        assert_eq!(
            result.stdout,
            expected.as_bytes(),
            "workers={workers}\n{source}"
        );
        assert!(result.stderr.is_empty(), "runtime diagnostic: {result:?}");
    }
}
fn rejects(label: &str, source: &str) {
    for workers in [1, 8] {
        let scratch = Scratch::new();
        let input = scratch.source(source);
        let output = scratch.path("must-not-exist");
        for check in [true, false] {
            let result = capture(
                &mut compiler(&input, &output, workers, check),
                &scratch,
                Duration::from_secs(20),
            );
            assert!(
                !result.status.success(),
                "{label}: accepted workers={workers}, check={check}\n{source}"
            );
            assert!(
                result.status.code().is_some(),
                "{label}: compiler terminated by signal: {result:?}"
            );
            assert!(!result.stderr.is_empty(), "{label}: missing diagnostic");
            assert!(
                !String::from_utf8_lossy(&result.stderr).contains("panicked at"),
                "{label}: compiler panic: {result:?}"
            );
            assert!(!output.exists(), "{label}: published executable");
        }
    }
}

#[test]
fn loop_values_continue_and_unit_break_build_check_and_execute() {
    runs(
        r#"
fn never_called()->bool { loop {continue;} }
fn builtin_never()->bool { !{return true;} }
fn main() {
    let mut n=0;
    let mut sum=0;
    let answer=loop {
        n=n+1;
        if n<3 { continue; }
        sum=sum+n;
        if n==5 { break sum; }
    };
    let flag=loop { break answer==12; };
    let unit=loop { break (); };
    loop { break }
    if flag && n==5 && builtin_never() { println!("loop values ok"); }
}
"#,
        "loop values ok\n",
    );
}

#[test]
fn nested_loop_and_while_control_targets_are_innermost() {
    runs(
        r#"
fn main() {
    let mut outer=0;
    let mut total=0;
    loop {
        outer=outer+1;
        let mut inner=0;
        let value=loop {
            inner=inner+1;
            if inner<3 { continue; }
            break outer+inner;
        };
        let mut w=0;
        while true {
            w=w+1;
            if w<2 { continue; }
            break;
        }
        total=total+value+w;
        if outer==3 { break; }
    }
    if total==21 && outer==3 { println!("nested targets ok"); }
}
"#,
        "nested targets ok\n",
    );
}

#[test]
fn break_and_continue_restore_pending_operand_and_argument_temporaries() {
    runs(
        r#"
fn pair(a:i32,b:i32)->i32 { a*10+b }
fn main() {
    let first=loop { 100+{break 7; 0}; };
    let second=loop { pair(4,{break 9;}); };
    let mut attempts=0;
    let third=loop {
        attempts=attempts+1;
        pair(99,{if attempts<3 {continue;} break 11;});
    };
    let mut operand_attempts=0;
    let fourth=loop {
        operand_attempts=operand_attempts+1;
        50+{if operand_attempts<3 {continue;} break 17; 0};
    };
    let mut w=0;
    while true {
        w=w+1;
        pair(55,{if w<3 {continue;} break;});
    }
    if first==7 && second==9 && third==11 && fourth==17 && attempts==3 && operand_attempts==3 && w==3 {
        println!("temporary cleanup ok");
    }
}
"#,
        "temporary cleanup ok\n",
    );
}

#[test]
fn inner_loop_preserves_outer_temporaries_and_break_operand_target() {
    runs(
        r#"
fn pair(a:i32,b:i32)->i32 { a*10+b }
fn main() {
    let first=10+loop {break 5;};
    let second=pair(7,loop {break 3;});
    let nested=loop { break loop { break 13; }; };
    let divergent_operand=loop { break {break 19;}; };
    let mut n=0;
    let fourth=100+loop {
        n=n+1;
        if n<3 {continue;}
        break 17;
    };
    if first==15 && second==73 && nested==13 && divergent_operand==19 && fourth==117 && n==3 {
        println!("outer temporaries ok");
    }
}
"#,
        "outer temporaries ok\n",
    );
}

#[test]
fn while_condition_allows_a_nested_loop_with_its_own_control_target() {
    runs(
        r#"
fn main() {
    let mut count=0;
    while {
        let mut attempts=0;
        loop {
            attempts=attempts+1;
            if attempts<2 {continue;}
            break count<3;
        }
    } { count=count+1; }
    if count==3 { println!("condition loop ok"); }
}
"#,
        "condition loop ok\n",
    );
}

#[test]
fn explicit_semicolon_discards_nonunit_block_if_and_loop_values() {
    runs(
        r#"
fn main() {
    { 9 };
    if true { 7 } else { 8 };
    loop {break 11;};
    let result={ loop {break 13;} };
    if result==13 { println!("discard ok"); }
}
"#,
        "discard ok\n",
    );
}

#[test]
fn loop_break_types_and_unused_bodies_are_checked_even_when_unreachable() {
    for (label, source) in [
        (
            "mixed scalar break types",
            "fn main(){let x=loop {if true {break 1;} break false;};}",
        ),
        (
            "unreachable break type",
            "fn main(){let x=loop {break 1; break true;};}",
        ),
        (
            "bare break conflicts with scalar",
            "fn main(){let x=loop {if true {break 1;} break;};}",
        ),
        (
            "break value disagrees with annotation",
            "fn main(){let x:bool=loop {break 1;};}",
        ),
        ("nonunit loop body", "fn main(){loop {1}}"),
        ("nonunit while body", "fn main(){while false {1}}"),
        (
            "unused invalid loop body",
            "fn unused(){loop {break 1; break false;};} fn main(){}",
        ),
        (
            "unused invalid break operand",
            "fn main(){loop {break 1; break missing;};}",
        ),
        (
            "omitted semicolon block discard",
            "fn main(){{9} println!(\"bad\");}",
        ),
        (
            "omitted semicolon if discard",
            "fn main(){if true {7}else{8} println!(\"bad\");}",
        ),
        (
            "omitted semicolon loop discard",
            "fn main(){loop {break 1;} println!(\"bad\");}",
        ),
    ] {
        rejects(label, source);
    }
}

#[test]
fn while_break_values_outside_control_and_self_condition_control_reject() {
    for (label, source) in [
        ("while scalar break", "fn main(){while true {break 1;}}"),
        (
            "while unit break value",
            "fn main(){while true {break ();}}",
        ),
        (
            "while block unit break value",
            "fn main(){while true {break {};}}",
        ),
        ("break outside loop", "fn main(){break;}"),
        ("continue outside loop", "fn main(){continue;}"),
        (
            "unused break outside loop",
            "fn unused(){break;} fn main(){}",
        ),
        (
            "break in own while condition",
            "fn main(){while {break; true} {}}",
        ),
        (
            "continue in own while condition",
            "fn main(){while {continue; true} {}}",
        ),
        (
            "while condition cannot target outer loop break",
            "fn main(){loop {while {break; true} {} break;}}",
        ),
        (
            "while condition cannot target outer loop continue",
            "fn main(){loop {while {continue; true} {} break;}}",
        ),
    ] {
        rejects(label, source);
    }
}

#[test]
fn historical_labeled_loop_builds_checks_and_runs() {
    runs("fn main(){'outer: loop {break 'outer;}}", "");
}

#[test]
fn loop_milestone_boundaries_reject_without_silent_fallback() {
    // Intentional native subset boundaries, not stock Rust invalidity claims.
    for (label, source) in [
        ("labelled break", "fn main(){loop {break 'missing;}}"),
        ("labelled continue", "fn main(){loop {continue 'missing;}}"),
        ("while expression", "fn main(){let x=while false {}; }"),
        ("continue value", "fn main(){loop {continue 1;}}"),
    ] {
        rejects(label, source);
    }
}

#[test]
fn never_completion_does_not_make_invalid_scalar_operators_well_typed() {
    // External stock Rust 2024 oracle rejects pure Never operands here.
    // Unreachable tails typed as i32 are covered in the valid cleanup fixture.
    for (label, source) in [
        (
            "pure never arithmetic return",
            "fn f()->i32 {1+{return 3;}} fn main(){}",
        ),
        (
            "pure never equality return",
            "fn main(){let x=1=={return;};}",
        ),
        (
            "pure never arithmetic continue",
            "fn main(){loop {let x=1+{continue;};}}",
        ),
        (
            "pure never arithmetic break",
            "fn main(){let x=loop {1+{break 3;};};}",
        ),
    ] {
        rejects(label, source);
    }
}

#[test]
fn unreachable_typed_values_still_constrain_breaks_conditions_and_results() {
    for (label, source) in [
        (
            "noncompleting typed break versus local annotation",
            "fn main(){let x:bool=loop{break {return;0};};}",
        ),
        (
            "noncompleting typed break versus later break",
            "fn main(){let x=loop{break {return;0};break false;};}",
        ),
        (
            "unreachable nonunit loop body tail",
            "fn main(){loop{break 1;false};}",
        ),
        (
            "noncompleting i32 if condition",
            "fn main(){if {return;1} {}}",
        ),
        (
            "noncompleting i32 while condition",
            "fn main(){while {return;1} {}}",
        ),
        (
            "noncompleting bool call argument",
            "fn take(x:i32){} fn main(){take({return;false});}",
        ),
        (
            "noncompleting bool return operand",
            "fn f()->i32{return {return 3;false};} fn main(){}",
        ),
    ] {
        rejects(label, source);
    }
    // A pure Never operand has no concrete i32 constraint to conflict with
    // the later bool break. The early return means there is no output.
    runs("fn main(){let x=loop{break {return;};break false;};}", "");
}

fn stock(source: &str, expected: Option<&str>) {
    let scratch = Scratch::new();
    let input = scratch.source(source);
    let output = scratch.path("stock");
    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let result = capture(
        Command::new(rustc)
            .env_remove("RUSTC_BOOTSTRAP")
            .args(["--edition=2024", "-Cpanic=abort"])
            .arg(&input)
            .arg("-o")
            .arg(&output),
        &scratch,
        Duration::from_secs(30),
    );
    if let Some(expected) = expected {
        result.success(source);
        let result = capture(&mut Command::new(output), &scratch, Duration::from_secs(5));
        result.success(source);
        assert_eq!(result.stdout, expected.as_bytes(), "{source}");
    } else {
        assert!(
            !result.status.success(),
            "stock accepted invalid fixture: {source}"
        );
    }
}

#[test]
fn range_for_matches_stock_once_only_bounds_scope_and_control() {
    let source = r#"
fn bound(x:i32)->i32 { if x==2 {println!("two");} if x==4 {println!("four");} if x==6 {println!("six");} x }
fn main() {
    let i=2;
    let mut sum=0;
    for i in bound(i)..bound(6) {
        if i==3 {continue;}
        sum=sum+i;
        if i==4 {break;}
    }
    if i==2 && sum==6 {println!("scope sum ok");}
    for _ in bound(4)..bound(2) {println!("bad");}
    for _ in 3..3 {println!("bad");}
    let mut count=0;
    'outer: for n in 0..=3 {
        for j in 0..3 {
            if j==1 {continue 'outer;}
            count=count+n;
        }
    }
    if count==6 {println!("continue ok");}
    'stop: for _ in 0..5 {for _ in 0..5 {break 'stop;}}
    for mut x in 1..3 {x=x+10;if x==11 {println!("eleven");} if x==12 {println!("twelve");}}
}
"#;
    let expected = "two\nsix\nscope sum ok\nfour\ntwo\ncontinue ok\neleven\ntwelve\n";
    stock(source, Some(expected));
    runs(source, expected);
}

#[test]
fn inclusive_range_maxima_do_not_overflow_or_repeat_on_continue() {
    let source = r#"
fn main(){
    let mut count=0;
    for x in 254u8..=255u8 {if x==254u8 || x==255u8 {count=count+1;}continue;}
    for x in 126i8..=127i8 {if x==126i8 || x==127i8 {count=count+1;}}
    for x in 18446744073709551615u64..=18446744073709551615u64 {if x==18446744073709551615u64 {count=count+1;}}
    for x in -128i8..=-127i8 {if x== -128i8 || x== -127i8 {count=count+1;}}
    for x in 340282366920938463463374607431768211455u128..=340282366920938463463374607431768211455u128 {if x==340282366920938463463374607431768211455u128 {count=count+1;}}
    for x in 170141183460469231731687303715884105727i128..=170141183460469231731687303715884105727i128 {if x==170141183460469231731687303715884105727i128 {count=count+1;}}
    if count==9 {println!("maxima ok");}
    for _ in 2..=1 {println!("bad");}
}
"#;
    let expected = "maxima ok\n";
    stock(source, Some(expected));
    runs(source, expected);
}

#[test]
fn range_for_invalid_patterns_types_binding_scope_and_break_values() {
    for source in [
        "fn main(){for i in 0..3 {} let x=i;}",
        "fn main(){for i in i..3 {}}",
        "fn main(){for i in 0..3 {i=4;}}",
        "fn main(){for i in 0..3 {break 1;}}",
        "fn main(){for i in 0..3 {break ();}}",
        "fn main(){for i in 0u8..3i32 {}}",
        "fn main(){for i in false..true {}}",
        "fn main(){for 1 in 0..3 {}}",
        "fn main(){for i in 0..3 {1}}",
        "fn main(){for i in 3..0 {missing;}}",
        "fn main(){for i in 0..3 {continue 'missing;}}",
        "fn f()->i32{for _ in ({return 7;0}..3) {}} fn main(){}",
        "fn f()->i32{return 7;for _ in 0..3 {}} fn main(){}",
        "fn main(){for _ in ({return;0}..3) {missing;}}",
    ] {
        stock(source, None);
        rejects("invalid range for", source);
    }
}

#[test]
fn range_bounds_keep_enclosing_control_and_once_only_mutation() {
    let source = r#"
fn main(){
    let mut n=0;
    let mut end=4;
    let mut sum=0;
    for x in ({n=n+1;1}..{n=n+10;end}) {
        end=0;
        sum=sum+x;
    }
    if n==11 && sum==6 && end==0 {println!("bounds snapshot ok");}
    let mut attempts=0;
    'outer: loop {
        attempts=attempts+1;
        for _ in ({if attempts<3 {continue 'outer;} 0}..2) {break 'outer;}
    }
    if attempts==3 {println!("bound control ok");}
    for _ in ({return;}..3) {println!("bad");}
}
"#;
    let expected = "bounds snapshot ok\nbound control ok\n";
    stock(source, Some(expected));
    runs(source, expected);
}

#[test]
fn copy_array_for_matches_stock_snapshot_patterns_and_control() {
    let source = r#"
fn values()->[i32;3]{println!("array once");[2,3,4]}
fn main(){
    let mut values=[1,2,3];
    let mut sum=0;
    for x in values {values[1]=99;sum=sum+x;}
    if sum==6 && values[1]==99 {println!("array snapshot ok");}
    let mut total=0;
    for x in [2,3,4] {if x==3 {continue;} total=total+x;}
    if total==6 {println!("array continue ok");}
    let mut pairs=0;
    for (a,b) in [(1,2),(3,4)] {pairs=pairs+a+b;}
    for [a,b] in [[5,6],[7,8]] {pairs=pairs+a+b;}
    if pairs==36 {println!("array patterns ok");}
    let mut nested=0;
    'outer: for x in [1,2,3] {
        for y in [4,5] {if y==5 {continue 'outer;} nested=nested+x;}
    }
    if nested==6 {println!("array labels ok");}
    let mut first=0;
    for x in [7,8] {first=x;break;}
    if first==7 {println!("array break ok");}
    let x=9;
    for mut x in [x,10] {x=x+1;}
    if x==9 {println!("array scope ok");}
    for _ in crate::values() {}
}
"#;
    let expected = "array snapshot ok\narray continue ok\narray patterns ok\narray labels ok\narray break ok\narray scope ok\narray once\n";
    stock(source, Some(expected));
    runs(source, expected);
}

#[test]
fn copy_array_for_invalid_scope_mutability_refutability_and_unit_obligations() {
    for source in [
        "fn main(){for x in [1,2] {} let y=x;}",
        "fn main(){for x in [1,2] {x=3;}}",
        "fn main(){for 1 in [1,2] {}}",
        "fn main(){for (a,b) in [1,2] {}}",
        "fn main(){for x in [1,2] {break 7;}}",
        "fn main(){for x in [1,2] {true}}",
        "fn f()->i32{for _ in [1,2] {}} fn main(){}",
    ] {
        stock(source, None);
        rejects("invalid Copy-array iteration", source);
    }
}

#[test]
fn referenced_array_for_uses_real_shared_and_mutable_elements() {
    let source = r#"
fn main(){
    let mut values=[1,2,3];
    let mut sum=0;
    for x in &values {sum=sum+*x;}
    if sum==6 {println!("shared ok");}
    for x in &mut values {*x=*x+10;}
    if values[0]==11 && values[1]==12 && values[2]==13 {println!("mutation ok");}
    let mut sum=0;
    for x in &values {if *x==12 {continue;}sum=sum+*x;}
    if sum==24 {println!("continue ok");}
    for x in &mut values {*x=5;break;}
    values[2]=7;
    if values[0]==5 && values[2]==7 {println!("last use ok");}
}
"#;
    let expected = "shared ok\nmutation ok\ncontinue ok\nlast use ok\n";
    stock(source, Some(expected));
    runs(source, expected);
}

#[test]
fn referenced_array_for_keeps_once_only_control() {
    let source = r#"
fn update(a:&mut [i32;3])->i32 {
    for x in a {*x=*x+1;if *x==3 {return *x;}}
    0
}
fn main(){
    let mut values=[1,2,3];
    let mut hits=0;
    for x in ({hits=hits+1;&mut values}) {*x=*x+1;}
    if hits==1 && values[2]==4 {println!("source once ok");}
    let mut count=0;
    'outer: for x in &mut values {
        *x=*x+1;
        for y in [0,1] {if y==1 {continue 'outer;}count=count+1;}
    }
    if count==3 && values[2]==5 {println!("labels ok");}
    let mut more=[1,2,3];
    if update(&mut more)==3 && more[0]==2 && more[1]==3 && more[2]==3 {println!("return ok");}
}
"#;
    let expected = "source once ok\nlabels ok\nreturn ok\n";
    stock(source, Some(expected));
    runs(source, expected);
}

#[test]
fn referenced_array_for_empty_and_singleton_keep_body_types() {
    let source = r#"
fn main(){
    let mut empty:[i32;0]=[];
    for x in &empty {let n=*x;println!("bad");}
    for x in &mut empty {*x=7;println!("bad");}
    let mut one=[8];
    for x in &mut one {*x=*x+1;}
    if one[0]==9 {println!("empty singleton ok");}
}
"#;
    stock(source, Some("empty singleton ok\n"));
    runs(source, "empty singleton ok\n");
}

#[test]
fn referenced_array_for_preserves_body_identity_and_captured_addresses() {
    let source = r#"
fn identity<'a>(x:&'a mut i32)->&'a mut i32 {x}
fn main(){
    let mut values=[1;40];
    let mut count=0;
    for x in &mut values {let before=*x;let y=identity(x);*y=before+count;count=count+1;}
    if count==40 && values[0]==1 && values[39]==40 {println!("single body identity ok");}
    let mut last=&values[0];
    for x in &values {last=x;break;}
    if *last==1 {println!("captured address ok");}
    values[39]=99;
    if values[39]==99 {println!("escaped last use ok");}
    let units=[(),(),()];
    let mut count=0;
    for x in &units {let unit=*x;count=count+1;}
    if count==3 {println!("unit elements ok");}
}
"#;
    let expected =
        "single body identity ok\ncaptured address ok\nescaped last use ok\nunit elements ok\n";
    stock(source, Some(expected));
    runs(source, expected);
}

#[test]
fn referenced_array_for_scalar_widths_and_drop_exits_match_stock() {
    let source = r#"
struct Token {id:i32}
impl Drop for Token {fn drop(&mut self){if self.id==1 {println!("drop one");}else{println!("drop two");}}}
fn early(a:&[i32;2]) {for x in a {let token=Token{id:*x};return;}}
fn main(){
    let mut bytes=[255u8,254u8];
    for x in &mut bytes {*x=*x-1u8;}
    let mut signed=[-128i8,-2i8];
    for x in &mut signed {*x=*x+1i8;}
    let mut wide=[170141183460469231731687303715884105726i128];
    for x in &mut wide {*x=*x+1i128;}
    let mut floats=[1.5f32,2.5f32];
    for x in &mut floats {*x=*x+1.0f32;}
    let mut flags=[true,false];
    for x in &mut flags {*x=!*x;}
    let mut chars=['a','b'];
    for x in &mut chars {*x='z';}
    if bytes[0]==254u8 && bytes[1]==253u8 && signed[0]==-127i8 && signed[1]==-1i8 && wide[0]==170141183460469231731687303715884105727i128 && floats[0]==2.5f32 && floats[1]==3.5f32 && !flags[0] && flags[1] && chars[1]=='z' {println!("scalar layouts ok");}
    let values=[1,2];
    for x in &values {let token=Token{id:*x};if *x==1 {continue;}break;}
    early(&values);
    println!("exits done");
}
"#;
    let expected = "scalar layouts ok\ndrop one\ndrop two\ndrop one\nexits done\n";
    stock(source, Some(expected));
    runs(source, expected);
}

#[test]
fn referenced_array_for_rejects_aliasing_mutability_and_escape() {
    for source in [
        "fn main(){let a=[1,2];for x in &a {*x=3;}}",
        "fn main(){let a=[1,2];for x in &mut a {*x=3;}}",
        "fn main(){let mut a=[1,2];for x in &a {a[0]=3;let n=*x;}}",
        "fn main(){let mut a=[1,2];for x in &mut a {let n=a[0];*x=n;}}",
        "fn main(){let mut a=[1,2];for x in &mut a {let y=&*x;*x=3;let n=*y;}}",
        "fn main(){let mut a=[1,2];for x in &mut a {let y=&mut *x;*x=3;*y=4;}}",
        "fn main(){let a=[1,2];for x in &a {} let n=*x;}",
        "fn main(){let mut a=[1,2];let r=&a;for x in &mut a {*x=3;} let n=(*r)[0];}",
        "fn main(){let mut a=[1,2];let r=&mut a;for x in r {*x=3;}for x in r {*x=4;}}",
        "fn bad<'a>()->&'a i32{let a=[1,2];for x in &a {return x;}loop{};}fn main(){}",
        "fn main(){let a:[i32;0]=[];for x in &a {*x=3;}}",
        "fn main(){let a=[1,2];for x in &a {true}}",
    ] {
        stock(source, None);
        rejects("invalid referenced-array element loan", source);
    }
}

#[test]
fn array_iteration_unsupported_boundaries_are_stock_valid_not_invalid_claims() {
    for source in [
        "struct Item{x:i32} fn main(){for item in [Item{x:1},Item{x:2}] {}}",
        "fn scan(a:&mut [i32]){for x in a {*x=3;}}fn main(){let mut a=[1,2];scan(&mut a);}",
        "fn main(){for x in [1,2].into_iter() {}}",
    ] {
        stock(source, Some(""));
        rejects("explicit unsupported iterator boundary", source);
    }
    // Formerly unsupported boundaries that now build and match stock.
    for (source, expected) in [
        (
            r#"struct Item{x:i32} fn main(){let a=[Item{x:1},Item{x:2}];for x in &a {let n=x.x;println!("{}",n);}}"#,
            "1\n2\n",
        ),
        (
            r#"fn main(){let a=[1,2];let mut last=&a[0];for x in &a {last=x;}let n=*last;println!("{}",n);}"#,
            "2\n",
        ),
        (
            r#"fn main(){for x in [1,2].iter() {println!("{}",x);}}"#,
            "1\n2\n",
        ),
        (
            r#"fn relay<'a>(s:&'a [i32])->&'a [i32]{s}fn main(){for x in relay(&[4,5]){let n=*x;println!("{}",n);}}"#,
            "4\n5\n",
        ),
    ] {
        stock(source, Some(expected));
        runs(source, expected);
    }
}

#[test]
fn shared_slice_for_matches_stock_origin_control_and_once_only_source() {
    let source = r#"
fn first<'a>(a:&'a [i32])->&'a i32 {for x in a {return x;}loop{};}
fn sum(a:&[i32])->i32 {let mut n=0;for x in a {if *x==2 {continue;}n=n+*x;}n}
fn main(){
    let mut a=[1,2,3];
    let s:&[i32]=&a;
    let mut hits=0;
    let mut n=0;
    for x in ({hits=hits+1;s}) {n=n+*x;}
    if hits==1 && n==6 && sum(s)==4 && *first(s)==1 {println!("slice origin once ok");}
    let mut last=&a[0];
    for x in s {last=x;break;}
    if *last==1 {println!("slice captured ok");}
    {let inner:&[i32]=&a;for x in inner {last=x;break;}}
    if *last==1 {println!("slice descriptor scope ok");}
    a[0]=9;
    if a[0]==9 {println!("slice last use ok");}
    let empty:[i32;0]=[];
    let e:&[i32]=&empty;
    for x in e {println!("bad");}
    let units=[(),(),()];
    let u:&[()]=&units;
    let mut count=0;
    for x in u {let unit=*x;count=count+1;}
    if count==3 {println!("slice empty unit ok");}
}
"#;
    let expected = "slice origin once ok\nslice captured ok\nslice descriptor scope ok\nslice last use ok\nslice empty unit ok\n";
    stock(source, Some(expected));
    runs(source, expected);
}

#[test]
fn shared_slice_for_rejects_root_alias_mutation_and_escaping_elements() {
    for source in [
        "fn main(){let mut a=[1,2];let s:&[i32]=&a;for x in s {a[1]=4;let n=*x;}}",
        "fn main(){let a=[1,2];let s:&[i32]=&a;for x in s {*x=4;}}",
        "fn main(){let a:[i32;0]=[];let s:&[i32]=&a;for x in s {*x=4;}}",
        "fn bad<'a>()->&'a i32{let a=[1,2];let s:&[i32]=&a;for x in s {return x;}loop{};}fn main(){}",
        "fn main(){let a=[1,2];let mut last=&a[0];{let b=[3,4];let s:&[i32]=&b;for x in s {last=x;break;}}let n=*last;}",
        "fn bad<'a,'b>(a:&'a [i32],b:&'b [i32])->&'a i32{for x in b {return x;}loop{};}fn main(){}",
        "fn bad<'a>(a:&'a [i32])->&'a mut i32{for x in a {return x;}loop{};}fn main(){}",
        "fn first<'a>(a:&'a [i32])->&'a i32{for x in a{return x;}loop{};}fn main(){let mut a=[1,2];let s:&[i32]=&a;let r=first(s);a[0]=8;let n=*r;}",
        "fn bad()->&'static i32{let a=[1,2];let s:&[i32]=&a;for x in s{return x;}loop{};}fn main(){}",
    ] {
        stock(source, None);
        rejects("invalid shared slice iteration loan", source);
    }
}

#[test]
fn shared_slice_for_temporary_argument_and_iterable_descriptor() {
    let source = r#"
fn scan(s:&[i32])->i32 {let mut n=0;for x in s {n=n+*x;}n}
fn main(){
    let mut n=scan(&[4,5]);
    if n==9 {println!("slice temporary ok");}
    let values=[6,7];
    let mut calls=0;
    for x in ({calls=calls+1;let s:&[i32]=&values;s}) {if *x==7 {break;}n=n+*x;}
    if calls==1 && n==15 {println!("slice call once ok");}
}
"#;
    let expected = "slice temporary ok\nslice call once ok\n";
    stock(source, Some(expected));
    runs(source, expected);
}
