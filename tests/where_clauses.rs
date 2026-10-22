//! Type-parameter where-clause stock/native corpus (experiments/where_clauses_correctness.py).
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
            let path = base.join(format!("jr-where-{}-{epoch}-{id}", std::process::id()));
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
    assert!(
        fs::metadata(&stdout).unwrap().len() <= 65536,
        "stdout capture exceeded64KiB"
    );
    assert!(
        fs::metadata(&stderr).unwrap().len() <= 65536,
        "stderr capture exceeded64KiB"
    );
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
fn runs_with_exit(source: &str, expected: &str, expected_exit: i32) {
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
        assert_eq!(
            result.status.code(),
            Some(expected_exit),
            "runtime status failure: {result:?}\n{source}"
        );
        assert_eq!(
            result.stdout,
            expected.as_bytes(),
            "workers={workers}\n{source}"
        );
        if expected_exit == 0 {
            assert!(
                result.stderr.is_empty(),
                "unexpected runtime stderr: {result:?}"
            );
        }
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
                result.status.code() == Some(1),
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

fn runs(source: &str, expected: &str) {
    runs_with_exit(source, expected, 0);
}

#[test]
fn accepts_free_where_single() {
    runs(
        r###"struct S{x:i32}struct W<A>{x:A}trait T{fn v(self)->i32;}trait U{fn u(self)->i32;}impl T for S{fn v(self)->i32{self.x}}impl U for S{fn u(self)->i32{self.x*10}}fn g<X>(x:X)->i32 where X:T{x.v()}fn run()->i32{g(S{x:3})}
fn main(){if run()==3{println!("where oracle ok");}}
"###,
        "where oracle ok\n",
    );
}

#[test]
fn accepts_free_where_trailing_comma_multi() {
    runs(
        r###"struct S{x:i32}struct W<A>{x:A}trait T{fn v(self)->i32;}trait U{fn u(self)->i32;}impl T for S{fn v(self)->i32{self.x}}impl U for S{fn u(self)->i32{self.x*10}}fn h<X,Y>(x:X,y:Y)->i32 where X:T,Y:T+U,{<X as T>::v(x)+y.u()}fn run()->i32{h(S{x:1},S{x:2})}
fn main(){if run()==21{println!("where oracle ok");}}
"###,
        "where oracle ok\n",
    );
}

#[test]
fn accepts_inline_and_where_same_bound() {
    runs(
        r###"struct S{x:i32}struct W<A>{x:A}trait T{fn v(self)->i32;}trait U{fn u(self)->i32;}impl T for S{fn v(self)->i32{self.x}}impl U for S{fn u(self)->i32{self.x*10}}fn g<X:T>(x:X)->i32 where X:T{x.v()}fn run()->i32{g(S{x:4})}
fn main(){if run()==4{println!("where oracle ok");}}
"###,
        "where oracle ok\n",
    );
}

#[test]
fn accepts_inline_and_where_split_bounds() {
    runs(
        r###"struct S{x:i32}struct W<A>{x:A}trait T{fn v(self)->i32;}trait U{fn u(self)->i32;}impl T for S{fn v(self)->i32{self.x}}impl U for S{fn u(self)->i32{self.x*10}}fn k<X:T>(x:X,y:X)->i32 where X:U{x.v()+y.u()}fn run()->i32{k(S{x:1},S{x:2})}
fn main(){if run()==21{println!("where oracle ok");}}
"###,
        "where oracle ok\n",
    );
}

#[test]
fn accepts_conditional_impl_where() {
    runs(
        r###"struct S{x:i32}struct W<A>{x:A}trait T{fn v(self)->i32;}trait U{fn u(self)->i32;}impl T for S{fn v(self)->i32{self.x}}impl U for S{fn u(self)->i32{self.x*10}}impl<A> T for W<A> where A:T{fn v(self)->i32{self.x.v()+1}}fn g<X>(x:X)->i32 where X:T{x.v()}fn run()->i32{g(W::<W<S>>{x:W::<S>{x:S{x:1}}})}
fn main(){if run()==3{println!("where oracle ok");}}
"###,
        "where oracle ok\n",
    );
}

#[test]
fn accepts_inherent_impl_where_method() {
    runs(
        r###"struct S{x:i32}struct W<A>{x:A}trait T{fn v(self)->i32;}trait U{fn u(self)->i32;}impl T for S{fn v(self)->i32{self.x}}impl U for S{fn u(self)->i32{self.x*10}}impl<A> W<A> where A:T{fn get(self)->i32{self.x.v()}}fn run()->i32{(W::<S>{x:S{x:5}}).get()}
fn main(){if run()==5{println!("where oracle ok");}}
"###,
        "where oracle ok\n",
    );
}

#[test]
fn accepts_method_where_on_impl_param() {
    runs(
        r###"struct S{x:i32}struct W<A>{x:A}trait T{fn v(self)->i32;}trait U{fn u(self)->i32;}impl T for S{fn v(self)->i32{self.x}}impl U for S{fn u(self)->i32{self.x*10}}impl<A> W<A>{fn get(self)->i32 where A:T{self.x.v()}}fn run()->i32{(W::<S>{x:S{x:6}}).get()}
fn main(){if run()==6{println!("where oracle ok");}}
"###,
        "where oracle ok\n",
    );
}

#[test]
fn accepts_associated_fn_where_forwarding() {
    runs(
        r###"struct S{x:i32}struct W<A>{x:A}trait T{fn v(self)->i32;}trait U{fn u(self)->i32;}impl T for S{fn v(self)->i32{self.x}}impl U for S{fn u(self)->i32{self.x*10}}impl<A> W<A>{fn make(a:A)->W<A> where A:U{W{x:a}}fn take(self)->A{self.x}}fn run()->i32{W::<S>::make(S{x:2}).take().u()}
fn main(){if run()==20{println!("where oracle ok");}}
"###,
        "where oracle ok\n",
    );
}

#[test]
fn accepts_where_forwarded_to_bounded_callee() {
    runs(
        r###"struct S{x:i32}struct W<A>{x:A}trait T{fn v(self)->i32;}trait U{fn u(self)->i32;}impl T for S{fn v(self)->i32{self.x}}impl U for S{fn u(self)->i32{self.x*10}}fn inner<X:T>(x:X)->i32{x.v()}fn outer<X>(x:X)->i32 where X:T{inner(x)}fn run()->i32{outer(S{x:8})}
fn main(){if run()==8{println!("where oracle ok");}}
"###,
        "where oracle ok\n",
    );
}

#[test]
fn accepts_where_explicit_type_args() {
    runs(
        r###"struct S{x:i32}struct W<A>{x:A}trait T{fn v(self)->i32;}trait U{fn u(self)->i32;}impl T for S{fn v(self)->i32{self.x}}impl U for S{fn u(self)->i32{self.x*10}}fn g<X>(x:X)->i32 where X:U{x.u()}fn run()->i32{g::<S>(S{x:9})}
fn main(){if run()==90{println!("where oracle ok");}}
"###,
        "where oracle ok\n",
    );
}

#[test]
fn accepts_where_bound_qualified_path() {
    runs(
        r###"struct S{x:i32}struct W<A>{x:A}trait T{fn v(self)->i32;}trait U{fn u(self)->i32;}impl T for S{fn v(self)->i32{self.x}}impl U for S{fn u(self)->i32{self.x*10}}fn g<X>(x:X)->i32 where X:T{X::v(x)}fn run()->i32{g(S{x:7})}
fn main(){if run()==7{println!("where oracle ok");}}
"###,
        "where oracle ok\n",
    );
}

#[test]
fn accepts_where_unused_parameter_bound() {
    runs(
        r###"struct S{x:i32}struct W<A>{x:A}trait T{fn v(self)->i32;}trait U{fn u(self)->i32;}impl T for S{fn v(self)->i32{self.x}}impl U for S{fn u(self)->i32{self.x*10}}fn g<X,Y>(x:X)->i32 where Y:U,X:T{x.v()}fn run()->i32{g::<S,S>(S{x:2})}
fn main(){if run()==2{println!("where oracle ok");}}
"###,
        "where oracle ok\n",
    );
}

#[test]
fn rejects_where_missing_impl_concrete() {
    rejects(
        "where_missing_impl_concrete",
        r###"struct S{x:i32}struct W<A>{x:A}trait T{fn v(self)->i32;}trait U{fn u(self)->i32;}impl T for S{fn v(self)->i32{self.x}}impl U for S{fn u(self)->i32{self.x*10}}fn g<X>(x:X)->i32 where X:T{x.v()}fn run()->i32{g(5i32)}
fn main(){if run()==7{println!("where oracle ok");}}
"###,
    );
}

#[test]
fn rejects_where_unproven_in_generic_caller() {
    rejects(
        "where_unproven_in_generic_caller",
        r###"struct S{x:i32}struct W<A>{x:A}trait T{fn v(self)->i32;}trait U{fn u(self)->i32;}impl T for S{fn v(self)->i32{self.x}}impl U for S{fn u(self)->i32{self.x*10}}fn g<X>(x:X)->i32 where X:T{7}fn bad<X>(x:X)->i32{g(x)}fn run()->i32{7}
fn main(){if run()==7{println!("where oracle ok");}}
"###,
    );
}

#[test]
fn rejects_where_unknown_parameter() {
    rejects(
        "where_unknown_parameter",
        r###"struct S{x:i32}struct W<A>{x:A}trait T{fn v(self)->i32;}trait U{fn u(self)->i32;}impl T for S{fn v(self)->i32{self.x}}impl U for S{fn u(self)->i32{self.x*10}}fn g<X>(x:X)->i32 where Y:T{7}fn run()->i32{7}
fn main(){if run()==7{println!("where oracle ok");}}
"###,
    );
}

#[test]
fn rejects_where_unknown_trait() {
    rejects(
        "where_unknown_trait",
        r###"struct S{x:i32}struct W<A>{x:A}trait T{fn v(self)->i32;}trait U{fn u(self)->i32;}impl T for S{fn v(self)->i32{self.x}}impl U for S{fn u(self)->i32{self.x*10}}fn g<X>(x:X)->i32 where X:Missing{7}fn run()->i32{7}
fn main(){if run()==7{println!("where oracle ok");}}
"###,
    );
}

#[test]
fn rejects_where_wrong_bound_method() {
    rejects(
        "where_wrong_bound_method",
        r###"struct S{x:i32}struct W<A>{x:A}trait T{fn v(self)->i32;}trait U{fn u(self)->i32;}impl T for S{fn v(self)->i32{self.x}}impl U for S{fn u(self)->i32{self.x*10}}fn g<X>(x:X)->i32 where X:U{x.v()}fn run()->i32{7}
fn main(){if run()==7{println!("where oracle ok");}}
"###,
    );
}

#[test]
fn rejects_impl_where_unsatisfied_method() {
    rejects(
        "impl_where_unsatisfied_method",
        r###"struct S{x:i32}struct W<A>{x:A}trait T{fn v(self)->i32;}trait U{fn u(self)->i32;}impl T for S{fn v(self)->i32{self.x}}impl U for S{fn u(self)->i32{self.x*10}}impl<A> T for W<A> where A:U{fn v(self)->i32{7}}struct Z{x:i32}impl T for Z{fn v(self)->i32{self.x}}fn run()->i32{(W::<Z>{x:Z{x:1}}).v()}
fn main(){if run()==7{println!("where oracle ok");}}
"###,
    );
}

#[test]
fn rejects_where_unused_body_move() {
    rejects(
        "where_unused_body_move",
        r###"struct S{x:i32}struct W<A>{x:A}trait T{fn v(self)->i32;}trait U{fn u(self)->i32;}impl T for S{fn v(self)->i32{self.x}}impl U for S{fn u(self)->i32{self.x*10}}fn g<X>(x:X)->(X,X) where X:T{(x,x)}fn run()->i32{7}
fn main(){if run()==7{println!("where oracle ok");}}
"###,
    );
}

#[test]
fn rejects_method_where_unsatisfied() {
    rejects(
        "method_where_unsatisfied",
        r###"struct S{x:i32}struct W<A>{x:A}trait T{fn v(self)->i32;}trait U{fn u(self)->i32;}impl T for S{fn v(self)->i32{self.x}}impl U for S{fn u(self)->i32{self.x*10}}impl<A> W<A>{fn get(self)->i32 where A:U{7}}struct Z{x:i32}impl T for Z{fn v(self)->i32{self.x}}fn run()->i32{(W::<Z>{x:Z{x:1}}).get()}
fn main(){if run()==7{println!("where oracle ok");}}
"###,
    );
}

#[test]
fn accepts_self_where_predicate() {
    runs(
        r###"struct S{x:i32}struct W<A>{x:A}trait T{fn v(self)->i32;}trait U{fn u(self)->i32;}impl T for S{fn v(self)->i32{self.x}}impl U for S{fn u(self)->i32{self.x*10}}trait V{fn w(self)->i32 where Self:Sized;}impl V for S{fn w(self)->i32{self.x}}fn run()->i32{S{x:3}.w()}
fn main(){if run()==3{println!("where oracle ok");}}
"###,
        "where oracle ok\n",
    );
}

#[test]
fn accepts_relaxed_sized_where() {
    runs(
        r###"struct S{x:i32}struct W<A>{x:A}trait T{fn v(self)->i32;}trait U{fn u(self)->i32;}impl T for S{fn v(self)->i32{self.x}}impl U for S{fn u(self)->i32{self.x*10}}fn g<X>()->i32 where X:?Sized{3}fn run()->i32{g::<S>()}
fn main(){if run()==3{println!("where oracle ok");}}
"###,
        "where oracle ok\n",
    );
}

#[test]
fn accepts_concrete_subject_where() {
    runs(
        r###"struct S{x:i32}struct W<A>{x:A}trait T{fn v(self)->i32;}trait U{fn u(self)->i32;}impl T for S{fn v(self)->i32{self.x}}impl U for S{fn u(self)->i32{self.x*10}}fn g()->i32 where S:T{S{x:3}.v()}fn run()->i32{g()}
fn main(){if run()==3{println!("where oracle ok");}}
"###,
        "where oracle ok\n",
    );
}

#[test]
fn boundary_higher_ranked_where() {
    rejects(
        "higher_ranked_where",
        r###"struct S{x:i32}struct W<A>{x:A}trait T{fn v(self)->i32;}trait U{fn u(self)->i32;}impl T for S{fn v(self)->i32{self.x}}impl U for S{fn u(self)->i32{self.x*10}}fn g<X>(x:X)->i32 where for<'a> X:T{x.v()}fn run()->i32{g(S{x:3})}
fn main(){if run()==3{println!("where oracle ok");}}
"###,
    );
}
