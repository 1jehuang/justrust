//! B1 genuine nonescaping local references and borrowing acceptance.
//! All 35 exact fixtures were stock-classified with rustc 1.98.1, edition 2024.
//! Stock is an external scratch oracle only and is never invoked by these tests.
//! References in signatures/aggregates, reference referents and rvalue borrows remain boundaries.
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
            let path = base.join(format!("jr-borrowing-{}-{epoch}-{id}", std::process::id()));
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
fn accepts_shared_dereference() {
    runs(
        r#"fn run()->i32{let x=7;let r:&i32=&x;*r}
fn main() { if run() == 7 { println!("borrow oracle ok"); } }
"#,
        "borrow oracle ok\n",
    );
}

#[test]
fn accepts_shared_reference_copy() {
    runs(
        r#"fn run()->i32{let x=7;let r=&x;let s=r;*r+*s}
fn main() { if run() == 14 { println!("borrow oracle ok"); } }
"#,
        "borrow oracle ok\n",
    );
}

#[test]
fn accepts_immutable_carrier_mutable_referent() {
    runs(
        r#"fn run()->i32{let mut x=3;let r=&mut x;*r=*r+4;*r}
fn main() { if run() == 7 { println!("borrow oracle ok"); } }
"#,
        "borrow oracle ok\n",
    );
}

#[test]
fn accepts_shared_nll_last_use() {
    runs(
        r#"fn run()->i32{let mut x=3;let r=&x;let n=*r;x=7;n+x}
fn main() { if run() == 10 { println!("borrow oracle ok"); } }
"#,
        "borrow oracle ok\n",
    );
}

#[test]
fn accepts_mutable_nll_last_use() {
    runs(
        r#"fn run()->i32{let mut x=3;let r=&mut x;*r=7;x}
fn main() { if run() == 7 { println!("borrow oracle ok"); } }
"#,
        "borrow oracle ok\n",
    );
}

#[test]
fn accepts_mutable_reference_move() {
    runs(
        r#"fn run()->i32{let mut x=3;let r=&mut x;let s=r;*s=7;x}
fn main() { if run() == 7 { println!("borrow oracle ok"); } }
"#,
        "borrow oracle ok\n",
    );
}

#[test]
fn accepts_explicit_mutable_reborrow() {
    runs(
        r#"fn run()->i32{let mut x=3;let r=&mut x;let s=&mut *r;*s=7;*r}
fn main() { if run() == 7 { println!("borrow oracle ok"); } }
"#,
        "borrow oracle ok\n",
    );
}

#[test]
fn accepts_explicit_shared_reborrow() {
    runs(
        r#"fn run()->i32{let mut x=3;let r=&mut x;let s=&*r;let n=*s;*r=7;n+*r}
fn main() { if run() == 10 { println!("borrow oracle ok"); } }
"#,
        "borrow oracle ok\n",
    );
}

#[test]
fn accepts_disjoint_struct_fields() {
    runs(
        r#"struct S{a:i32,b:i32} fn run()->i32{let mut p=S{a:3,b:7};let a=&mut p.a;let b=&mut p.b;*a=*a+1;*a+*b}
fn main() { if run() == 11 { println!("borrow oracle ok"); } }
"#,
        "borrow oracle ok\n",
    );
}

#[test]
fn accepts_branch_reference_origins() {
    runs(
        r#"fn choose(b:bool)->i32{let x=3;let y=7;let r:&i32;if b{r=&x;}else{r=&y;}*r} fn run()->i32{choose(true)+choose(false)}
fn main() { if run() == 10 { println!("borrow oracle ok"); } }
"#,
        "borrow oracle ok\n",
    );
}

#[test]
fn accepts_loop_last_use() {
    runs(
        r#"fn run()->i32{let mut x=0;let mut i=0;while i<3{let r=&mut x;*r=*r+1;i=i+1;}x}
fn main() { if run() == 3 { println!("borrow oracle ok"); } }
"#,
        "borrow oracle ok\n",
    );
}

#[test]
fn accepts_shared_array_elements() {
    runs(
        r#"fn run()->i32{let a=[3,7];let r=&a[0];let s=&a[1];*r+*s}
fn main() { if run() == 10 { println!("borrow oracle ok"); } }
"#,
        "borrow oracle ok\n",
    );
}

#[test]
fn accepts_copy_aggregate_dereference() {
    runs(
        r#"fn run()->i32{let p=(3,7);let r=&p;let q=*r;q.0*10+q.1}
fn main() { if run() == 37 { println!("borrow oracle ok"); } }
"#,
        "borrow oracle ok\n",
    );
}

#[test]
fn accepts_lexical_mutable_loan_scope() {
    runs(
        r#"fn run()->i32{let mut x=3;{let r=&mut x;*r=7;}x}
fn main() { if run() == 7 { println!("borrow oracle ok"); } }
"#,
        "borrow oracle ok\n",
    );
}

#[test]
fn rejects_invalid_mutable_carrier_shared_cannot_write() {
    // Stock rejects this fixture with E0594.
    rejects(
        "mutable_carrier_shared_cannot_write",
        r#"fn run()->i32{let mut x=3;let mut r=&x;*r=7;*r}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_mutable_borrow_immutable_owner() {
    // Stock rejects this fixture with E0596.
    rejects(
        "mutable_borrow_immutable_owner",
        r#"fn run()->i32{let x=3;let r=&mut x;*r}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_loop_renewed_shared_call_result_conflicts() {
    // Stock rejects these with E0502 and E0597.
    rejects(
        "loop_call_result_then_push",
        "fn main(){let mut v=vec![1.0f64,3.0];let mut it=v.iter();let mut best=it.next().unwrap();\
loop{match it.next(){Some(x)=>{if *x>*best{best=x;}v.push(1.0);}None=>break}}println!(\"{}\",best);}\n",
    );
    rejects(
        "loop_call_result_outlives_local",
        "fn id(x:&String)->&String{x}\nfn main(){let a=String::from(\"a\");let mut best=id(&a);let mut i=0;\
while i<3{let s=String::from(\"b\");best=id(&s);i+=1;}println!(\"{}\",best);}\n",
    );
}

#[test]
fn borrow_of_reference_carrier_keeps_inner_loan_live() {
    // `&'r Option<&'a T>` implies `'a: 'r`. Stock rejects each with E0502/E0506.
    rejects(
        "borrowed_option_ref_then_assign",
        "fn main(){let mut x=1u32;let a=Some(&x);let r=&a;x=2;println!(\"{:?}\",r);}\n",
    );
    rejects(
        "borrowed_get_result_then_pop",
        "fn main(){let mut v:Vec<u32>=vec![1];let a=v.get(0);let r=&a;v.pop();println!(\"{:?}\",r);}\n",
    );
    rejects(
        "format_get_and_pop",
        "fn main(){let mut v:Vec<u32>=vec![1];println!(\"{:?} {:?}\",v.get(0),v.pop());}\n",
    );
    runs(
        "fn main(){let mut x=1u32;let a=Some(&x);let r=&a;println!(\"{:?}\",r);x=2;\
let mut v:Vec<u32>=vec![4];println!(\"{:?} {}\",v.get(0),v.len());v.pop();println!(\"{}\",x+v.len() as u32);}\n",
        "Some(1)\nSome(4) 1\n2\n",
    );
}

#[test]
fn rejects_invalid_shared_mutable_conflict() {
    // Stock rejects this fixture with E0502.
    rejects(
        "shared_mutable_conflict",
        r#"fn run()->i32{let mut x=3;let r=&x;let s=&mut x;*r+*s}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_two_mutable_loans() {
    // Stock rejects this fixture with E0499.
    rejects(
        "two_mutable_loans",
        r#"fn run()->i32{let mut x=3;let r=&mut x;let s=&mut x;*r+*s}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_owner_write_while_loan_live() {
    // Stock rejects this fixture with E0506.
    rejects(
        "owner_write_while_loan_live",
        r#"fn run()->i32{let mut x=3;let r=&x;x=7;*r}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_owner_move_while_loan_live() {
    // Stock rejects this fixture with E0505.
    rejects(
        "owner_move_while_loan_live",
        r#"struct S{n:i32} fn run()->i32{let x=S{n:7};let r=&x;let y=x;(*r).n+y.n}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_noncopy_move_through_shared() {
    // Stock rejects this fixture with E0507.
    rejects(
        "noncopy_move_through_shared",
        r#"struct S{n:i32} fn run()->i32{let x=S{n:7};let r=&x;let y=*r;y.n}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_mutable_reference_moved_carrier() {
    // Stock rejects this fixture with E0382.
    rejects(
        "mutable_reference_moved_carrier",
        r#"fn run()->i32{let mut x=3;let r=&mut x;let s=r;*s=7;*r}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_array_index_loans_overlap() {
    // Stock rejects this fixture with E0499.
    rejects(
        "array_index_loans_overlap",
        r#"fn run()->i32{let mut a=[3,7];let r=&mut a[0];let s=&mut a[1];*r+*s}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_referent_storage_dead() {
    // Stock rejects this fixture with E0597.
    rejects(
        "referent_storage_dead",
        r#"fn run()->i32{let r:&i32;{let x=7;r=&x;}*r}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_parent_access_while_child_live() {
    // Stock rejects this fixture with E0506.
    rejects(
        "parent_access_while_child_live",
        r#"fn run()->i32{let mut x=3;let r=&mut x;let s=&mut *r;*r=7;*s}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_loop_referent_storage_dead() {
    // Stock rejects this fixture with E0597.
    rejects(
        "loop_referent_storage_dead",
        r#"fn run()->i32{let x=0;let mut r:&i32=&x;let mut i=0;while i<2{let y=i;r=&y;i=i+1;}*r}
fn main() { run(); }
"#,
    );
}

#[test]
fn accepts_reference_parameter_result() {
    runs(
        r#"fn pass(r:&i32)->&i32{r} fn run()->i32{let x=7;*pass(&x)}
fn main() { if run() == 7 { println!("borrow oracle ok"); } }
"#,
        "borrow oracle ok\n",
    );
}

#[test]
fn accepts_reference_aggregate_field() {
    // Lifetime-parameterized nominals with reference fields (G3).
    runs(
        r#"struct R<'a>{r:&'a i32} fn run()->i32{let x=7;let r=R{r:&x};*r.r}
fn main() { if run() == 7 { println!("borrow oracle ok"); } }
"#,
        "borrow oracle ok\n",
    );
}

#[test]
fn accepts_reference_to_reference() {
    // Unchanged source independently passes stock and native workers 1/8.
    runs(
        r#"fn run()->i32{let x=7;let r=&x;let rr=&r;**rr}
fn main() { if run() == 7 { println!("borrow oracle ok"); } }
"#,
        "borrow oracle ok\n",
    );
}

#[test]
fn accepts_lifetime_extended_rvalue_borrow() {
    runs(
        r#"fn run()->i32{let r=&7;*r}
fn main() { if run() == 7 { println!("borrow oracle ok"); } }
"#,
        "borrow oracle ok\n",
    );
}

#[test]
fn accepts_parent_handle_dies_child_lives() {
    runs(
        r#"fn run()->i32{let mut x=3;let s:&mut i32;{let r=&mut x;s=&mut *r;}*s=7;x}
fn main() { if run() == 7 { println!("borrow oracle ok"); } }
"#,
        "borrow oracle ok\n",
    );
}

#[test]
fn accepts_shared_carrier_reassignment() {
    runs(
        r#"fn run()->i32{let x=3;let y=7;let mut r=&x;let n=*r;r=&y;n+*r}
fn main() { if run() == 10 { println!("borrow oracle ok"); } }
"#,
        "borrow oracle ok\n",
    );
}

#[test]
fn rejects_invalid_shared_copy_survives_carrier_overwrite() {
    // Stock rejects this fixture with E0506.
    rejects(
        "shared_copy_survives_carrier_overwrite",
        r#"fn run()->i32{let mut x=3;let y=7;let mut r=&x;let s=r;r=&y;x=9;*s+*r}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_shared_child_freezes_parent_write() {
    // Stock rejects this fixture with E0506.
    rejects(
        "shared_child_freezes_parent_write",
        r#"fn run()->i32{let mut x=3;let r=&mut x;let s=&*r;*r=7;*s}
fn main() { run(); }
"#,
    );
}

#[test]
fn accepts_deref_lhs_resolved_after_rhs() {
    runs(
        r#"fn run()->i32{let mut x=1;let mut y=2;let mut r=&mut x;*r={r=&mut y;3};let w:usize=4294967296;let p=&w;if *p==4294967296{x+y}else{0}}
fn main() { if run() == 4 { println!("borrow oracle ok"); } }
"#,
        "borrow oracle ok\n",
    );
}

#[test]
fn accepts_generic_body_entry_with_closed_static_key() {
    runs(
        r#"use std::collections::HashMap;
struct Bus { subs: HashMap<&'static str, Vec<i32>> }
impl Bus { fn on<T: Default>(&mut self, name: &'static str, _t: T) { self.subs.entry(name).or_default().push(1); } }
fn main() { let mut b = Bus { subs: HashMap::new() }; b.on("a", 0u8); b.on("a", 1u8); if b.subs["a"].len() == 2 { println!("borrow oracle ok"); } }
"#,
        "borrow oracle ok\n",
    );
}

#[test]
fn accepts_static_key_map_with_boxed_values() {
    runs(
        r#"use std::collections::HashMap;
fn main() { let mut m: HashMap<&'static str, Vec<Box<dyn Fn(i32) -> i32>>> = HashMap::new(); m.entry("a").or_default().push(Box::new(|x| x + 1)); if m.get("a").map(|v| v[0](1)) == Some(2) { println!("borrow oracle ok"); } }
"#,
        "borrow oracle ok\n",
    );
}

#[test]
fn rejects_generic_body_entry_alias_conflict() {
    rejects(
        "generic_entry_alias",
        r#"use std::collections::HashMap;
fn on<T: Default>(m: &mut HashMap<&'static str, Vec<i32>>, _t: T) { let v = m.entry("a").or_default(); m.clear(); v.push(1); }
fn main() {}
"#,
    );
}

#[test]
fn rejects_frame_local_key_in_static_map() {
    rejects(
        "frame_local_static_key",
        r#"use std::collections::BTreeMap;
fn on(m: &mut BTreeMap<&'static str, i32>) { let s = String::from("a"); m.entry(s.as_str()).or_insert(0); }
fn main() {}
"#,
    );
}

#[test]
fn accepts_annotated_static_key_map_passed_to_generic_fn() {
    runs(
        r#"use std::collections::HashMap;
fn on<T: Default>(m: &mut HashMap<&'static str, Vec<i32>>, name: &'static str, _t: T) { m.entry(name).or_default().push(1); }
fn main() { let mut m: HashMap<&'static str, Vec<i32>> = HashMap::new(); on(&mut m, "a", 0u8); on(&mut m, "a", 1u8); if m["a"].len() == 2 { println!("borrow oracle ok"); } }
"#,
        "borrow oracle ok\n",
    );
}

#[test]
fn rejects_frame_local_key_passed_to_generic_static_param() {
    rejects(
        "generic_static_param_local_key",
        r#"use std::collections::HashMap;
fn on<T: Default>(m: &mut HashMap<&'static str, Vec<i32>>, name: &'static str, _t: T) { m.entry(name).or_default().push(1); }
fn main() { let mut m: HashMap<&'static str, Vec<i32>> = HashMap::new(); let s = String::from("a"); on(&mut m, s.as_str(), 0u8); }
"#,
    );
}

#[test]
fn accepts_generic_method_through_returned_mut_reference() {
    runs(
        r#"use std::collections::HashMap;
fn pick<T>(v: &mut Vec<T>) -> &mut Vec<T> { v }
fn reset<T>(v: &mut Vec<T>) { pick(v).clear(); }
fn add<T>(v: &mut Vec<T>, x: T) { pick(v).push(x); }
fn group<T: Clone>(xs: &[(u8, T)]) -> HashMap<u8, Vec<T>> {
    let mut m: HashMap<u8, Vec<T>> = HashMap::new();
    for (k, v) in xs { m.entry(*k).or_default().push(v.clone()); }
    m
}
fn group_with<T: Clone>(xs: &[(u8, T)]) -> HashMap<u8, Vec<T>> {
    let mut m: HashMap<u8, Vec<T>> = HashMap::new();
    for (k, v) in xs { m.entry(*k).or_insert_with(Vec::new).push(v.clone()); }
    m
}
fn main() {
    let mut v = vec![1, 2];
    add(&mut v, 3);
    println!("{:?}", v);
    reset(&mut v);
    println!("{:?}", v);
    let mut s = vec![String::from("a")];
    add(&mut s, String::from("b"));
    println!("{:?}", s);
    let g = group(&[(1u8, 10i32), (2, 20), (1, 30)]);
    println!("{:?} {:?}", g[&1], g[&2]);
    let h = group_with(&[(3u8, String::from("x")), (3, String::from("y"))]);
    println!("{:?}", h[&3]);
}
"#,
        "[1, 2, 3]\n[]\n[\"a\", \"b\"]\n[10, 30] [20]\n[\"x\", \"y\"]\n",
    );
}

const PUT_REFERENCE_HOLDING_SLOT: &str = "fn put<V>(v: &mut Vec<V>, x: V) -> &mut V { v.push(x); let n = v.len() - 1; match v.get_mut(n) { Some(r) => r, None => panic!() } }\n";

#[test]
fn accepts_generic_returned_slot_of_reference_holding_vec() {
    runs(
        &format!(
            "{PUT_REFERENCE_HOLDING_SLOT}{}",
            r#"fn main() {
    let s = String::from("hi");
    let t = String::from("yo");
    let mut v: Vec<Vec<&str>> = Vec::new();
    put(&mut v, Vec::new()).push(&s);
    put(&mut v, vec![t.as_str()]).push(&s);
    println!("{:?}", v);
}
"#
        ),
        "[[\"hi\"], [\"yo\", \"hi\"]]\n",
    );
}

#[test]
fn accepts_entry_or_insert_with_reference_holding_vec() {
    runs(
        r#"use std::collections::BTreeMap;
struct E<'a, V> { m: &'a mut Vec<V> }
impl<'a, V> E<'a, V> {
    fn ins<F: FnOnce() -> V>(self, g: F) -> &'a mut V {
        let m = self.m; let x = g(); m.push(x); let n = m.len() - 1;
        match m.get_mut(n) { Some(r) => r, None => panic!() }
    }
}
fn main() {
    let s = String::from("hi");
    let mut v: Vec<Vec<&str>> = Vec::new();
    E { m: &mut v }.ins(Vec::new).push(&s);
    println!("{:?}", v);
    let words = vec!["apple", "avocado", "bean"];
    let mut m: BTreeMap<char, Vec<&str>> = BTreeMap::new();
    for w in &words { m.entry(w.chars().next().unwrap()).or_insert_with(Vec::new).push(w); }
    let mut n: BTreeMap<char, Vec<&str>> = BTreeMap::new();
    for w in &words { n.entry('x').or_insert(Vec::new()).push(w); }
    println!("{:?} {:?}", m, n);
}
"#,
        "[[\"hi\"]]\n{'a': [\"apple\", \"avocado\"], 'b': [\"bean\"]} {'x': [\"apple\", \"avocado\", \"bean\"]}\n",
    );
}

#[test]
fn vec_into_iter_reference_elements_and_drops_match_stock() {
    runs(
        "struct D(i32);
impl Drop for D { fn drop(&mut self) { println!(\"drop {}\", self.0); } }
fn main() {
    let a = vec![1i32, 2, 3];
    let r: Vec<&i32> = vec![&a[0], &a[2]];
    let mut s = 0i32;
    for x in r { s += *x; }
    let mut it = vec![D(1), D(2), D(3)].into_iter();
    let first = it.next().unwrap();
    let last = it.next_back().unwrap();
    drop(it);
    println!(\"{} {} {}\", s, first.0, last.0);
    let strs = vec![String::from(\"p\"), String::from(\"q\")];
    let mut back = strs.into_iter();
    while let Some(w) = back.next_back() { println!(\"{}\", w); }
}
",
        "drop 2\n4 1 3\nq\np\ndrop 3\ndrop 1\n",
    );
}

#[test]
fn rejects_references_escaping_through_raw_pointer_storage() {
    // Stock rejects each with E0597. vec::IntoIter keeps its elements behind a
    // raw pointer, so these must never be accepted.
    rejects(
        "into_iter item escape",
        "fn main(){let item;{let a=vec![1i32];item=vec![&a[0]].into_iter().next().unwrap();}println!(\"{}\",item);}\n",
    );
    rejects(
        "into_iter iterator escape",
        "fn main(){let mut it;{let a=vec![1i32];it=vec![&a[0]].into_iter();}println!(\"{:?}\",it.next());}\n",
    );
    rejects(
        "flat_map borrowed inner escape",
        "fn main(){let item;{let a=vec![1i32,2i32];item=(0i32..1i32).flat_map(|_|a.iter()).next().unwrap();}println!(\"{}\",item);}\n",
    );
    // Stock E0597: references stored behind Rc's raw pointee.
    rejects(
        "rc reference escape",
        "use std::rc::Rc;fn main(){let r;{let a=5i32;r=Rc::new(&a);}println!(\"{}\",r);}\n",
    );
    rejects(
        "rc clone reference escape",
        "use std::rc::Rc;fn main(){let r;{let a=String::from(\"s\");let rc=Rc::new(&a);r=Rc::clone(&rc);}println!(\"{}\",r);}\n",
    );
}

#[test]
fn rejects_entry_or_insert_with_reference_holding_vec_misuse() {
    rejects(
        "short-lived entry element",
        r#"use std::collections::BTreeMap;
fn main() {
    let mut b: BTreeMap<char, Vec<&str>> = BTreeMap::new();
    { let s = String::from("apple"); b.entry('a').or_insert_with(Vec::new).push(&s); }
    println!("{:?}", b);
}
"#,
    );
    rejects(
        "short-lived closure-made entry",
        r#"use std::collections::BTreeMap;
fn main() {
    let mut b: BTreeMap<char, Vec<&str>> = BTreeMap::new();
    { let s = String::from("apple"); b.entry('a').or_insert_with(|| vec![s.as_str()]).push("x"); }
    println!("{:?}", b);
}
"#,
    );
}

#[test]
fn rejects_generic_returned_slot_of_reference_holding_vec_misuse() {
    rejects(
        "short-lived element",
        &format!(
            "{PUT_REFERENCE_HOLDING_SLOT}{}",
            r#"fn main() {
    let mut v: Vec<Vec<&str>> = Vec::new();
    { let s = String::from("hi"); put(&mut v, Vec::new()).push(&s); }
    println!("{:?}", v);
}
"#
        ),
    );
    rejects(
        "mutation while stored",
        &format!(
            "{PUT_REFERENCE_HOLDING_SLOT}{}",
            r#"fn main() {
    let mut s = String::from("hi");
    let mut v: Vec<Vec<&str>> = Vec::new();
    put(&mut v, Vec::new()).push(&s);
    s.push('x');
    println!("{:?} {}", v, s);
}
"#
        ),
    );
}

#[test]
fn rejects_generic_returned_mut_reference_conflicts() {
    rejects(
        "generic_returned_mut_e0499",
        "fn pick<T>(v: &mut Vec<T>) -> &mut Vec<T> { v } fn unused<T>(v: &mut Vec<T>, x: T) { let r = pick(v); v.clear(); r.push(x); } fn main() {}",
    );
    rejects(
        "generic_returned_mut_e0502",
        "fn pick<T>(v: &mut Vec<T>) -> &mut Vec<T> { v } fn unused<T>(v: &mut Vec<T>, x: T) { let r = pick(v); let n = v.len(); r.push(x); let _ = n; } fn main() {}",
    );
}

#[test]
fn accepts_generic_recursive_enum_boxing_its_owner() {
    runs(
        r#"#[derive(Debug)]
enum L<X> { Nil, Cons(Box<L<X>>, X) }
impl<X: std::fmt::Debug> L<X> {
    fn wrap(self, x: X) -> L<X> { L::Cons(Box::new(self), x) }
    fn len(&self) -> usize { match self { L::Nil => 0, L::Cons(rest, _) => 1 + rest.len() } }
}
fn boxed<X>(l: L<X>) -> Box<L<X>> { Box::new(l) }
fn main() {
    let l = L::Nil.wrap(String::from("a")).wrap(String::from("b"));
    println!("{} {:?}", l.len(), l);
    println!("{:?}", boxed(L::Nil.wrap(3u8)));
}
"#,
        "2 Cons(Cons(Nil, \"a\"), \"b\")\nCons(Nil, 3)\n",
    );
}

#[test]
fn rejects_generic_recursive_enum_owner_reuse() {
    rejects(
        "generic_recursive_enum_e0382",
        "enum L<X> { Nil, Cons(Box<L<X>>, X) } fn unused<X>(l: L<X>, x: X, y: X) -> (L<X>, L<X>) { (L::Cons(Box::new(l), x), L::Cons(Box::new(l), y)) } fn main() {}",
    );
    rejects(
        "generic_recursive_enum_e0382_payload",
        "enum L<X> { Nil, Cons(Box<L<X>>, X) } fn unused<X>(l: L<X>, x: X) -> (L<X>, X) { (L::Cons(Box::new(l), x), x) } fn main() {}",
    );
}
