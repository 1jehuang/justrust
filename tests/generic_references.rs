//! Frozen exact32 generic references and borrowed receiver acceptance corpus.
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
                "jr-generic-references-{}-{epoch}-{id}",
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
fn rejects(label: &str, source: &str, semantic: bool) {
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
            if semantic {
                let diagnostic = String::from_utf8_lossy(&result.stderr).to_lowercase();
                for marker in [
                    "panic",
                    "parse",
                    "unexpected",
                    "unsupported",
                    "not supported",
                ] {
                    assert!(
                        !diagnostic.contains(marker),
                        "{label}: nonsemantic rejection: {result:?}"
                    );
                }
            }
            assert!(!output.exists(), "{label}: published executable");
        }
    }
}

fn runs(source: &str, expected: &str) {
    runs_with_exit(source, expected, 0);
}

#[test]
fn accepts_generic_shared_identity_and_rigid_forwarding() {
    runs(
        r###"fn id<T>(x:&T)->&T{x}fn forward<T>(x:&T)->&T{id::<T>(x)}fn run()->i32{let x=7;*forward::<i32>(&x)}
fn main(){if run()==7{println!("generic references oracle ok");}}
"###,
        "generic references oracle ok\n",
    );
}

#[test]
fn accepts_generic_mutable_identity_and_write() {
    runs(
        r###"fn id<T>(x:&mut T)->&mut T{x}fn run()->i32{let mut x=3;let r=id::<i32>(&mut x);*r=7;x}
fn main(){if run()==7{println!("generic references oracle ok");}}
"###,
        "generic references oracle ok\n",
    );
}

#[test]
fn accepts_shared_carrier_copy_noncopy_referent() {
    runs(
        r###"struct S{x:i32}fn twice<T>(r:&T)->&T{let a=r;let b=r;let _=a;b}fn run()->i32{let s=S{x:7};let r=twice::<S>(&s);r.x}
fn main(){if run()==7{println!("generic references oracle ok");}}
"###,
        "generic references oracle ok\n",
    );
}

#[test]
fn accepts_reference_free_product_referents_and_disjoint_fields() {
    runs(
        r###"struct S{x:i32,y:i32}fn id<T>(x:&mut T)->&mut T{x}fn run()->i32{let mut s=S{x:1,y:2};let r=id::<i32>(&mut s.x);s.y=3;*r=4;let mut a=[1,2];*id::<i32>(&mut a[0])=4;let mut p=(1,2);*id::<i32>(&mut p.0)=4;s.x+s.y+a[0]+p.0-8}
fn main(){if run()==7{println!("generic references oracle ok");}}
"###,
        "generic references oracle ok\n",
    );
}

#[test]
fn accepts_t_u_coincidence_preserves_declared_region_authority() {
    runs(
        r###"fn first<'a,'b,T,U>(x:&'a T,y:&'b U)->&'a T{let _=y;x}fn run()->i32{let x=7;let r;{let y=3;r=first::<i32,i32>(&x,&y);}*r}
fn main(){if run()==7{println!("generic references oracle ok");}}
"###,
        "generic references oracle ok\n",
    );
}

#[test]
fn accepts_inherent_shared_receiver_preserves_owner() {
    runs(
        r###"struct S{x:i32}impl S{fn get(&self)->i32{self.x}}fn run()->i32{let s=S{x:3};let a=s.get();let r=&s;a+r.get()+1}
fn main(){if run()==7{println!("generic references oracle ok");}}
"###,
        "generic references oracle ok\n",
    );
}

#[test]
fn accepts_inherent_mut_receiver_write() {
    runs(
        r###"struct S{x:i32}impl S{fn set(&mut self,x:i32){self.x=x;}}fn run()->i32{let mut s=S{x:0};s.set(7);s.x}
fn main(){if run()==7{println!("generic references oracle ok");}}
"###,
        "generic references oracle ok\n",
    );
}

#[test]
fn accepts_receiver_output_elision_over_other_reference_input() {
    runs(
        r###"struct S{x:i32}impl S{fn get(&self,other:&i32)->&i32{let _=other;&self.x}}fn run()->i32{let s=S{x:7};let r;{let other=3;r=s.get(&other);}*r}
fn main(){if run()==7{println!("generic references oracle ok");}}
"###,
        "generic references oracle ok\n",
    );
}

#[test]
fn accepts_named_receiver_alpha_normalized_contract() {
    runs(
        r###"trait Get{fn get<'a>(&'a self)->&'a i32;}struct S{x:i32}impl Get for S{fn get<'z>(&'z self)->&'z i32{&self.x}}fn run()->i32{let s=S{x:7};*<S as Get>::get(&s)}
fn main(){if run()==7{println!("generic references oracle ok");}}
"###,
        "generic references oracle ok\n",
    );
}

#[test]
fn accepts_trait_projected_shared_mutable_returns_and_generic_forwarding() {
    runs(
        r###"trait Field{fn field(&self)->&i32;fn field_mut(&mut self)->&mut i32;}struct S{x:i32}impl Field for S{fn field(&self)->&i32{&self.x}fn field_mut(&mut self)->&mut i32{&mut self.x}}fn shared<T:Field>(x:&T)->&i32{x.field()}fn mutable<T:Field>(x:&mut T)->&mut i32{x.field_mut()}fn run()->i32{let mut s=S{x:3};*mutable::<S>(&mut s)=7;*shared::<S>(&s)}
fn main(){if run()==7{println!("generic references oracle ok");}}
"###,
        "generic references oracle ok\n",
    );
}

#[test]
fn accepts_conditional_trait_dispatch_and_static_trait_result_opacity() {
    runs(
        r###"trait Value{fn value(&self)->i32;}trait Take{fn take(x:&Self)->&i32;}struct S{x:i32}impl Value for S{fn value(&self)->i32{self.x}}impl Take for S{fn take(x:&Self)->&i32{&x.x}}struct Wrap<T>{x:T}impl<T:Value> Value for Wrap<T>{fn value(&self)->i32{self.x.value()}}fn read<T:Take>(x:&T)->&i32{T::take(x)}fn run()->i32{let s=S{x:3};let a=*read::<S>(&s);let w=Wrap::<S>{x:S{x:4}};a+w.value()}
fn main(){if run()==7{println!("generic references oracle ok");}}
"###,
        "generic references oracle ok\n",
    );
}

#[test]
fn accepts_genuine_reference_call_receiver_once() {
    runs(
        r###"struct S{x:i32}impl S{fn get(&self)->i32{self.x}}fn pick<'a>(s:&'a S,count:&mut i32)->&'a S{*count+=1;s}fn run()->i32{let s=S{x:6};let mut count=0;let x=pick(&s,&mut count).get();x+count}
fn main(){if run()==7{println!("generic references oracle ok");}}
"###,
        "generic references oracle ok\n",
    );
}

#[test]
fn accepts_receiver_and_explicit_arguments_once_ltr() {
    runs(
        r###"struct S{x:i32}impl S{fn set(&mut self,a:i32,b:i32){self.x=a+b;}}fn pick<'a>(s:&'a mut S,c:&mut i32)->&'a mut S{*c=*c*10+1;s}fn arg(c:&mut i32,x:i32)->i32{*c=*c*10+x;x}fn run()->i32{let mut s=S{x:0};let mut c=0;pick(&mut s,&mut c).set(arg(&mut c,2),arg(&mut c,3));if c==123{s.x+2}else{0}}
fn main(){if run()==7{println!("generic references oracle ok");}}
"###,
        "generic references oracle ok\n",
    );
}

#[test]
fn accepts_implicit_mut_receiver_shared_read_then_activation() {
    runs(
        r###"struct S{x:i32}impl S{fn get(&self)->i32{self.x}fn set(&mut self,x:i32){self.x=x;}}fn run()->i32{let mut s=S{x:6};s.set(s.get()+1);s.x}
fn main(){if run()==7{println!("generic references oracle ok");}}
"###,
        "generic references oracle ok\n",
    );
}

#[test]
fn accepts_implicit_mut_argument_reborrow_shared_read() {
    runs(
        r###"fn set(r:&mut i32,x:i32){*r=x;}fn run()->i32{let mut x=6;let r=&mut x;set(r,*r+1);x}
fn main(){if run()==7{println!("generic references oracle ok");}}
"###,
        "generic references oracle ok\n",
    );
}

#[test]
fn accepts_returned_mut_ref_last_use_and_per_iteration_two_phase_loop() {
    runs(
        r###"struct S{x:i32}impl S{fn get(&self)->i32{self.x}fn set(&mut self,x:i32)->&mut i32{self.x=x;&mut self.x}}fn run()->i32{let mut s=S{x:4};let mut i=0;while i<3{let r=s.set(s.get()+1);let _=*r;i+=1;}s.x}
fn main(){if run()==7{println!("generic references oracle ok");}}
"###,
        "generic references oracle ok\n",
    );
}

#[test]
fn rejects_invalid_unused_generic_body_move_through_deref() {
    rejects(
        "unused_generic_body_move_through_deref",
        r###"fn bad<T>(x:&T)->T{*x}fn run()->i32{7}
fn main(){run();}
"###,
        true,
    );
}

#[test]
fn rejects_invalid_wrong_result_region_despite_t_u_concrete_coincidence() {
    rejects(
        "wrong_result_region_despite_T_U_concrete_coincidence",
        r###"fn wrong<'a,'b,T,U>(x:&'a T,y:&'b U)->&'a U{let _=x;y}fn run()->i32{let x=7;let y=3;*wrong::<i32,i32>(&x,&y)}
fn main(){run();}
"###,
        true,
    );
}

#[test]
fn rejects_invalid_mutable_reborrow_parent_conflict_live_child() {
    rejects(
        "mutable_reborrow_parent_conflict_live_child",
        r###"fn id<T>(x:&mut T)->&mut T{x}fn run()->i32{let mut x=1;let r=&mut x;let child=id::<i32>(r);*r=3;*child}
fn main(){run();}
"###,
        true,
    );
}

#[test]
fn rejects_invalid_unused_impl_borrowed_receiver_kind_mismatch() {
    rejects(
        "unused_impl_borrowed_receiver_kind_mismatch",
        r###"trait Get{fn get(&self)->i32;}struct S{x:i32}impl Get for S{fn get(&mut self)->i32{self.x}}fn run()->i32{7}
fn main(){run();}
"###,
        true,
    );
}

#[test]
fn rejects_invalid_explicit_mut_receiver_immediate_conflict() {
    rejects(
        "explicit_mut_receiver_immediate_conflict",
        r###"struct S{x:i32}impl S{fn get(&self)->i32{self.x}fn set(&mut self,x:i32){self.x=x;}}fn run()->i32{let mut s=S{x:7};S::set(&mut s,s.get());7}
fn main(){run();}
"###,
        true,
    );
}

#[test]
fn rejects_invalid_explicit_mut_argument_immediate_conflict() {
    rejects(
        "explicit_mut_argument_immediate_conflict",
        r###"fn set(r:&mut i32,x:i32){*r=x;}fn run()->i32{let mut x=7;let r=&mut x;set(&mut *r,*r);7}
fn main(){run();}
"###,
        true,
    );
}

#[test]
fn rejects_invalid_reservation_blocks_argument_write() {
    rejects(
        "reservation_blocks_argument_write",
        r###"struct S{x:i32}impl S{fn set(&mut self,x:i32){self.x=x;}}fn run()->i32{let mut s=S{x:7};s.set({s.x=3;7});7}
fn main(){run();}
"###,
        true,
    );
}

#[test]
fn rejects_invalid_reservation_blocks_argument_move() {
    rejects(
        "reservation_blocks_argument_move",
        r###"struct S{x:i32}impl S{fn set(&mut self,x:i32){self.x=x;}}fn consume(s:S)->i32{s.x}fn run()->i32{let mut s=S{x:7};s.set(consume(s));7}
fn main(){run();}
"###,
        true,
    );
}

#[test]
fn rejects_invalid_overlapping_second_mutable_reservation() {
    rejects(
        "overlapping_second_mutable_reservation",
        r###"struct S{x:i32}impl S{fn set(&mut self,x:i32)->i32{self.x=x;x}}fn run()->i32{let mut s=S{x:7};s.set(s.set(3));7}
fn main(){run();}
"###,
        true,
    );
}

#[test]
fn rejects_invalid_shared_argument_loan_live_at_activation() {
    rejects(
        "shared_argument_loan_live_at_activation",
        r###"struct S{x:i32}impl S{fn set(&mut self,x:&i32){self.x=*x;}}fn run()->i32{let mut s=S{x:7};s.set(&s.x);7}
fn main(){run();}
"###,
        true,
    );
}

#[test]
fn rejects_invalid_returned_ref_keeps_activated_loan_live() {
    rejects(
        "returned_ref_keeps_activated_loan_live",
        r###"struct S{x:i32}impl S{fn field(&mut self)->&mut i32{&mut self.x}}fn run()->i32{let mut s=S{x:7};let r=s.field();s.x=3;*r}
fn main(){run();}
"###,
        true,
    );
}

#[test]
fn rejects_invalid_borrowed_result_owner_storage_dead() {
    rejects(
        "borrowed_result_owner_storage_dead",
        r###"struct S{x:i32}impl S{fn get(&self)->&i32{&self.x}}fn run()->i32{let r;{let s=S{x:7};r=s.get();}*r}
fn main(){run();}
"###,
        true,
    );
}

#[test]
fn rejects_legal_unsupported_reference_type_actual_and_hidden_reference_nominal() {
    rejects(
        "reference_type_actual_and_hidden_reference_nominal",
        r###"struct Wrap<T>{x:T}fn id<T>(x:T)->T{x}fn run()->i32{let x=7;let r=id::<&i32>(&x);let w=Wrap::<&i32>{x:r};*w.x}
fn main(){if run()==7{println!("generic references oracle ok");}}
"###,
        false,
    );
}

#[test]
fn rejects_legal_unsupported_nested_reference_or_reference_aggregate() {
    rejects(
        "nested_reference_or_reference_aggregate",
        r###"fn run()->i32{let x=7;let r=&x;let nested=&&x;let p=(r,nested);*p.0+**p.1-7}
fn main(){if run()==7{println!("generic references oracle ok");}}
"###,
        false,
    );
}

#[test]
fn accepts_owned_rvalue_borrowed_receiver_temporary() {
    runs(
        r###"struct S{x:i32}impl S{fn get(&self)->i32{self.x}fn bump(&mut self)->i32{self.x+=1;self.x}}fn run()->i32{S{x:7}.get()+S{x:6}.bump()-7}
fn main(){if run()==7{println!("generic references oracle ok");}}
"###,
        "generic references oracle ok\n",
    );
}

#[test]
fn rejects_reference_into_dropped_rvalue_receiver_temporary() {
    rejects(
        "dropped_rvalue_receiver_temporary",
        r###"struct S{x:i32}impl S{fn get(&self)->&i32{&self.x}}fn run()->i32{let r=S{x:7}.get();*r}
fn main(){if run()==7{println!("generic references oracle ok");}}
"###,
        true,
    );
}

#[test]
fn rejects_legal_unsupported_dynamic_index_borrow_or_autoderef_chain() {
    rejects(
        "dynamic_index_borrow_or_autoderef_chain",
        r###"struct S{x:i32}impl S{fn get(&self)->i32{self.x}}fn run()->i32{let a=[S{x:7}];let i=0usize;let r=&a[i];let rr=&r;rr.get()}
fn main(){if run()==7{println!("generic references oracle ok");}}
"###,
        false,
    );
}

#[test]
fn accepts_borrowed_value_temporaries_with_rust_scopes() {
    runs(
        r###"struct P{a:i32}struct W<T>(T);struct S;impl S{fn b(&self)->i32{3}}
fn f(p:&P)->i32{p.a}fn g(x:&mut i32)->i32{*x+=1;*x}fn mk(a:i32)->P{P{a}}fn s(x:&[i64])->i64{x[1]}fn u(_s:&S)->i32{4}
fn run()->i32{let x=1;let r=&P{a:x};let t=(&mk(2),);let w=[&P{a:3}];let m=&mut P{a:x};m.a+=1;
let mut i=0;while f(&P{a:i})<3{i+=1;}
let n=f(&mk(9))+f(&mk(1))+g(&mut 4)+(s(&[1,2]) as i32)+u(&S)+S.b();
r.a+t.0.a+w[0].a+m.a+i+n}
fn main(){if run()==1+2+3+2+3+10+5+2+4+3{println!("generic references oracle ok");}}
"###,
        "generic references oracle ok\n",
    );
}

#[test]
fn rejects_statement_temporary_borrow_escaping_its_statement() {
    for source in [
        r###"struct P{a:i32}fn id(p:&P)->&P{p}fn run(x:i32)->i32{let r=id(&P{a:x});r.a}
fn main(){if run(7)==7{println!("generic references oracle ok");}}
"###,
        r###"struct P{a:i32}fn run(x:i32)->i32{let r;r=&P{a:x};r.a}
fn main(){if run(7)==7{println!("generic references oracle ok");}}
"###,
    ] {
        rejects("statement_temporary_escape", source, true);
    }
}
