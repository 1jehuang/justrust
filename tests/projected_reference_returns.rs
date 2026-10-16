// Projected input reference returns, exact frozen stock-classified corpus.
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
            let path = base.join(format!(
                "jr-projected-reference-returns-{}-{epoch}-{id}",
                std::process::id()
            ));
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
fn accepts_shared_tuple_projection() {
    runs(
        r#"fn get<'a>(p:&'a(i32,i32))->&'a i32{&(*p).1} fn run()->i32{let p=(3,7);*get(&p)}
fn main() { if run() == 7 { println!("projected returns oracle ok"); } }
"#,
        "projected returns oracle ok\n",
    );
}

#[test]
fn accepts_mutable_tuple_projection() {
    runs(
        r#"fn get<'a>(p:&'a mut(i32,i32))->&'a mut i32{&mut (*p).1} fn run()->i32{let mut p=(3,4);let r=get(&mut p);*r=7;p.1}
fn main() { if run() == 7 { println!("projected returns oracle ok"); } }
"#,
        "projected returns oracle ok\n",
    );
}

#[test]
fn accepts_shared_struct_projection() {
    runs(
        r#"struct S{x:i32,y:i32} fn get<'a>(p:&'a S)->&'a i32{&(*p).y} fn run()->i32{let p=S{x:3,y:7};*get(&p)}
fn main() { if run() == 7 { println!("projected returns oracle ok"); } }
"#,
        "projected returns oracle ok\n",
    );
}

#[test]
fn accepts_mutable_struct_projection() {
    runs(
        r#"struct S{x:i32,y:i32} fn get<'a>(p:&'a mut S)->&'a mut i32{&mut (*p).y} fn run()->i32{let mut p=S{x:3,y:4};let r=get(&mut p);*r=7;p.y}
fn main() { if run() == 7 { println!("projected returns oracle ok"); } }
"#,
        "projected returns oracle ok\n",
    );
}

#[test]
fn accepts_shared_constant_array_projection() {
    runs(
        r#"fn get<'a>(p:&'a[i32;2])->&'a i32{&(*p)[1]} fn run()->i32{let p=[3,7];*get(&p)}
fn main() { if run() == 7 { println!("projected returns oracle ok"); } }
"#,
        "projected returns oracle ok\n",
    );
}

#[test]
fn accepts_mutable_constant_array_projection() {
    runs(
        r#"fn get<'a>(p:&'a mut[i32;2])->&'a mut i32{&mut (*p)[1]} fn run()->i32{let mut p=[3,4];let r=get(&mut p);*r=7;p[1]}
fn main() { if run() == 7 { println!("projected returns oracle ok"); } }
"#,
        "projected returns oracle ok\n",
    );
}

#[test]
fn accepts_nested_static_projection() {
    runs(
        r#"struct S{v:([i32;2],bool)} fn get<'a>(p:&'a S)->&'a i32{&(*p).v.0[1]} fn run()->i32{let p=S{v:([3,7],true)};*get(&p)}
fn main() { if run() == 7 { println!("projected returns oracle ok"); } }
"#,
        "projected returns oracle ok\n",
    );
}

#[test]
fn accepts_opaque_enum_whole_leaf() {
    runs(
        r#"enum E{A,B} struct S{e:E} fn get<'a>(p:&'a S)->&'a E{&(*p).e} fn run()->i32{let p=S{e:E::A};let r=get(&p);let _=r;7}
fn main() { if run() == 7 { println!("projected returns oracle ok"); } }
"#,
        "projected returns oracle ok\n",
    );
}

#[test]
fn accepts_private_field_public_getter() {
    runs(
        r#"mod m{pub struct S{x:i32} pub fn make()->S{S{x:7}} pub fn get<'a>(p:&'a S)->&'a i32{&(*p).x}} fn run()->i32{let p=m::make();*m::get(&p)}
fn main() { if run() == 7 { println!("projected returns oracle ok"); } }
"#,
        "projected returns oracle ok\n",
    );
}

#[test]
fn accepts_correlated_argument_layouts() {
    runs(
        r#"fn get<'a>(n:i32,x:&'a(i32,bool),y:&'a(bool,i32),b:bool)->&'a i32{if b{&(*x).0}else{&(*y).1}} fn run()->i32{let x=(3,true);let y=(false,7);*get(0,&x,&y,true)*10+*get(0,&x,&y,false)}
fn main() { if run() == 37 { println!("projected returns oracle ok"); } }
"#,
        "projected returns oracle ok\n",
    );
}

#[test]
fn accepts_nominal_leaf_identity() {
    runs(
        r#"struct A{x:i32} struct B{x:i32} struct S{a:A,b:B} fn get<'a>(p:&'a S)->&'a B{&(*p).b} fn run()->i32{let p=S{a:A{x:3},b:B{x:7}};(*get(&p)).x}
fn main() { if run() == 7 { println!("projected returns oracle ok"); } }
"#,
        "projected returns oracle ok\n",
    );
}

#[test]
fn accepts_forward_recursive_projection() {
    runs(
        r#"fn get<'a>(p:&'a(i32,i32),n:i32)->&'a i32{if n==0{&(*p).1}else{get(p,n-1)}} fn forward<'a>(p:&'a(i32,i32))->&'a i32{get(p,3)} fn run()->i32{let p=(3,7);*forward(&p)}
fn main() { if run() == 7 { println!("projected returns oracle ok"); } }
"#,
        "projected returns oracle ok\n",
    );
}

#[test]
fn accepts_high64_mutable_projection() {
    runs(
        r#"struct S{x:usize} fn get<'a>(p:&'a mut S)->&'a mut usize{&mut (*p).x} fn run()->i32{let mut p=S{x:3usize};let r=get(&mut p);*r=18446744073709551615usize;if p.x==18446744073709551615usize{7}else{0}}
fn main() { if run() == 7 { println!("projected returns oracle ok"); } }
"#,
        "projected returns oracle ok\n",
    );
}

#[test]
fn accepts_copy_aggregate_projection() {
    runs(
        r#"struct S{p:(i32,i32)} fn get<'a>(s:&'a mut S)->&'a mut(i32,i32){&mut (*s).p} fn run()->i32{let mut s=S{p:(3,4)};let r=get(&mut s);*r=(7,8);s.p.0+s.p.1}
fn main() { if run() == 15 { println!("projected returns oracle ok"); } }
"#,
        "projected returns oracle ok\n",
    );
}

#[test]
fn accepts_independent_short_dependency() {
    runs(
        r#"fn get<'a,'b>(p:&'a(i32,i32),q:&'b bool)->&'a i32{&(*p).1} fn run()->i32{let p=(3,7);let r:&i32;{let q=true;r=get(&p,&q);}*r}
fn main() { if run() == 7 { println!("projected returns oracle ok"); } }
"#,
        "projected returns oracle ok\n",
    );
}

#[test]
fn accepts_parent_resumes_after_lastuse() {
    runs(
        r#"fn get<'a>(p:&'a mut(i32,i32))->&'a mut i32{&mut (*p).1} fn run()->i32{let mut p=(3,4);let parent=&mut p;let r=get(parent);*r=7;(*parent).0=8;(*parent).0+(*parent).1}
fn main() { if run() == 15 { println!("projected returns oracle ok"); } }
"#,
        "projected returns oracle ok\n",
    );
}

#[test]
fn accepts_disjoint_actual_fields() {
    runs(
        r#"struct S{x:(i32,i32),y:(i32,i32)} fn get<'a>(p:&'a mut(i32,i32))->&'a mut i32{&mut (*p).1} fn run()->i32{let mut s=S{x:(1,3),y:(2,4)};let a=get(&mut s.x);let b=get(&mut s.y);*a=7;*b=8;s.x.1+s.y.1}
fn main() { if run() == 15 { println!("projected returns oracle ok"); } }
"#,
        "projected returns oracle ok\n",
    );
}

#[test]
fn rejects_invalid_private_field_direct_access() {
    // Stock rejects this fixture with E0616.
    rejects(
        "private_field_direct_access",
        r#"mod m{pub struct S{x:i32} pub fn make()->S{S{x:7}}} fn run()->i32{let p=m::make();p.x}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_distinct_nominal_same_layout() {
    // Stock rejects this fixture with E0308.
    rejects(
        "distinct_nominal_same_layout",
        r#"struct A{x:i32} struct B{x:i32} struct S{a:A} fn get<'a>(p:&'a S)->&'a B{&(*p).a} fn run()->i32{7}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_parent_write_while_result_live() {
    // Stock rejects this fixture with E0506.
    rejects(
        "parent_write_while_result_live",
        r#"fn get<'a>(p:&'a mut(i32,i32))->&'a mut i32{&mut (*p).1} fn run()->i32{let mut p=(3,4);let r=get(&mut p);p.0=7;*r}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_array_indices_conservatively_alias() {
    // Stock rejects this fixture with E0499.
    rejects(
        "array_indices_conservatively_alias",
        r#"fn get<'a>(p:&'a mut(i32,i32))->&'a mut i32{&mut (*p).1} fn run()->i32{let mut a=[(1,3),(2,4)];let x=get(&mut a[0]);let y=get(&mut a[1]);*x+*y}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_owned_local_projected_return() {
    // Stock rejects this fixture with E0515.
    rejects(
        "owned_local_projected_return",
        r#"fn get<'a>(p:&'a(i32,i32))->&'a i32{let q=(3,7);&q.1} fn run()->i32{7}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_unused_invalid_projected_callee() {
    // Stock rejects this fixture with E0515.
    rejects(
        "unused_invalid_projected_callee",
        r#"fn bad<'a>()->&'a i32{let q=(3,7);&q.1} fn run()->i32{7}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_same_region_different_pointee_dependency() {
    // Stock rejects this fixture with E0597.
    rejects(
        "same_region_different_pointee_dependency",
        r#"fn get<'a>(p:&'a(i32,i32),q:&'a bool)->&'a i32{&(*p).1} fn run()->i32{let p=(3,7);let r:&i32;{let q=true;r=get(&p,&q);}*r}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_missing_projected_outlives() {
    // Stock rejects this fixture for a missing lifetime outlives bound.
    rejects(
        "missing_projected_outlives",
        r#"fn get<'a,'b>(p:&'b(i32,i32))->&'a i32{&(*p).1} fn run()->i32{7}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_same_address_conflicting_authority() {
    // Stock rejects this fixture with E0499.
    rejects(
        "same_address_conflicting_authority",
        r#"fn get<'a>(p:&'a mut(i32,i32))->&'a mut i32{&mut (*p).1} fn run()->i32{let mut p=(3,4);let parent=&mut p;let r=get(parent);let q=&mut (*parent).1;*r+*q}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_legal_unsupported_enum_payload_projection() {
    // Stock accepts and runs this fixture, outside the bounded projected-return slice.
    rejects(
        "enum_payload_projection",
        r#"enum E{A(i32),B} fn get<'a>(p:&'a E)->&'a i32{match p{E::A(v)=>v,E::B=>loop{}}} fn run()->i32{let p=E::A(7);*get(&p)}
fn main() { if run() == 7 { println!("projected returns oracle ok"); } }
"#,
    );
}

#[test]
fn rejects_legal_unsupported_dynamic_index_projection() {
    // Stock accepts and runs this fixture, outside the bounded projected-return slice.
    rejects(
        "dynamic_index_projection",
        r#"fn get<'a>(p:&'a[i32;2],i:usize)->&'a i32{&(*p)[i]} fn run()->i32{let p=[3,7];*get(&p,1usize)}
fn main() { if run() == 7 { println!("projected returns oracle ok"); } }
"#,
    );
}

#[test]
fn rejects_legal_unsupported_borrow_computed_call_result() {
    // Stock accepts and runs this fixture, outside the bounded projected-return slice.
    rejects(
        "borrow_computed_call_result",
        r#"fn get<'a>(p:&'a(i32,i32))->&'a i32{&(*p).1} fn run()->i32{let p=(3,7);let r=&*get(&p);*r}
fn main() { if run() == 7 { println!("projected returns oracle ok"); } }
"#,
    );
}

#[test]
fn rejects_legal_unsupported_mutable_input_shared_projection() {
    // Stock accepts and runs this fixture, outside the bounded projected-return slice.
    rejects(
        "mutable_input_shared_projection",
        r#"fn get<'a>(p:&'a mut(i32,i32))->&'a i32{&(*p).1} fn run()->i32{let mut p=(3,7);*get(&mut p)}
fn main() { if run() == 7 { println!("projected returns oracle ok"); } }
"#,
    );
}
