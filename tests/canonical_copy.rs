//! Frozen exact32 canonical Copy/Clone acceptance corpus.
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
                "jr-canonical-copy-{}-{epoch}-{id}",
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
/// Debug-binary compile cap. Prelude growth raised empty-program CHECK from
/// 0.26s (6835f01fe) to ~13s (d180c7bee) unloaded; 20s failed under the
/// parallel suite. A cap is a hang guard, not a correctness check.
const COMPILE_SECONDS: u64 = 60;
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
            let result = capture(
                &mut compiler(&input, &output, workers, check),
                &scratch,
                COMPILE_SECONDS,
            );
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
            let result = capture(
                &mut compiler(&input, &output, workers, check),
                &scratch,
                COMPILE_SECONDS,
            );
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
            let diagnostic = String::from_utf8_lossy(&result.stderr).to_lowercase();
            for marker in ["panic", "parse", "unexpected"] {
                assert!(
                    !diagnostic.contains(marker),
                    "{label}: nonsemantic failure: {result:?}"
                );
            }
            if !semantic {
                assert!(
                    diagnostic.contains("unsupported") || diagnostic.contains("not supported"),
                    "{label}: legal exclusion requires explicit unsupported diagnostic: {result:?}"
                );
            }
            if semantic {
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
// Exact observed corpus SHA256: 3c3759705d094af4f4c53a53e8bcb3bc400da7e1d31a84e878f5e2708502b1d7
// Python companion preserves every frozen dictionary field verbatim.
struct Fixture {
    id: &'static str,
    name: &'static str,
    group: &'static str,
    source_sha256: &'static str,
    full_source: &'static str,
    expected_stdout: &'static str,
}

const FIXTURES: &[Fixture] = &[
    Fixture {
        id: "P01",
        name: "canonical_core_std_identity_and_shadowable_fake_names",
        group: "positive",
        source_sha256: "874f98bbb9cea3bce5fb93b1cb94c9e8fa8297390655320183683ff201a14625",
        full_source: r###"trait Copy{fn fake(&self)->i32;}trait Clone{fn fake_clone(&self)->i32;}struct S{v:i32}impl core::clone::Clone for S{fn clone(&self)->Self{S{v:self.v}}}impl core::marker::Copy for S{}fn twice<T:std::marker::Copy>(x:T)->(T,T){(x,x)}fn run()->i32{let s=S{v:3};let p=twice::<S>(s);p.0.v+p.1.v}
fn main(){if run()==6{println!("canonical copy oracle ok");}}
"###,
        expected_stdout: "canonical copy oracle ok\n",
    },
    Fixture {
        id: "P02",
        name: "true_supertrait_closure_and_default_sized_generic",
        group: "positive",
        source_sha256: "b2c3e747b8b1384b555d4e723f4b9fe4b69e0f6b232ae1ba91ae12045f29e040",
        full_source: r###"trait Base{fn base(&self)->i32;}trait Child:Base{fn child(&self)->i32;}struct S{v:i32}impl Base for S{fn base(&self)->i32{self.v}}impl Child for S{fn child(&self)->i32{self.v+1}}impl Clone for S{fn clone(&self)->Self{S{v:self.v}}}impl Copy for S{}fn via<T:Child>(x:&T)->i32{x.base()+x.child()}fn duplicate<T:Copy>(x:&T)->T{x.clone()}fn sized<T>(x:T)->T{x}fn run()->i32{let s=S{v:3};let c=duplicate::<S>(&s);c.v+via::<S>(&c)+sized::<i32>(1)}
fn main(){if run()==11{println!("canonical copy oracle ok");}}
"###,
        expected_stdout: "canonical copy oracle ok\n",
    },
    Fixture {
        id: "P03",
        name: "manual_copy_clone_pair_body_uses_copy_without_header_cycle",
        group: "positive",
        source_sha256: "3f811619fd070d9abbbf9a1e93efb40ac28645f43a146a9200c35766f6ad10f1",
        full_source: r###"struct S{v:i32}impl Clone for S{fn clone(&self)->Self{*self}}impl Copy for S{}fn run()->i32{let s=S{v:7};let a=s;let b=s.clone();a.v+b.v}
fn main(){if run()==14{println!("canonical copy oracle ok");}}
"###,
        expected_stdout: "canonical copy oracle ok\n",
    },
    Fixture {
        id: "P04",
        name: "conditional_nominal_copy_uses_only_declared_field_bounds",
        group: "positive",
        source_sha256: "c48140519fe4b0d9c3db6e2ae97f135be0e98fccea08ad56206bfbf1536f471e",
        full_source: r###"struct Wrap<T>{v:T}impl<T:Copy> Clone for Wrap<T>{fn clone(&self)->Self{*self}}impl<T:Copy> Copy for Wrap<T>{}fn twice<T:Copy>(x:Wrap<T>)->(Wrap<T>,Wrap<T>){(x,x)}fn run()->i32{let p=twice::<i32>(Wrap{v:8});p.0.v+p.1.v}
fn main(){if run()==16{println!("canonical copy oracle ok");}}
"###,
        expected_stdout: "canonical copy oracle ok\n",
    },
    Fixture {
        id: "P05",
        name: "shared_reference_copy_without_referent_copy_or_clone",
        group: "positive",
        source_sha256: "474b5df2824be8bbd3df09027355577675a8422429bf15f055ef1429f155e165",
        full_source: r###"struct Opaque{v:i32}fn run()->i32{let owner=Opaque{v:9};let r=&owner;let a=r;let b=r;a.v+b.v}
fn main(){if run()==18{println!("canonical copy oracle ok");}}
"###,
        expected_stdout: "canonical copy oracle ok\n",
    },
    Fixture {
        id: "P06",
        name: "primitive_unit_and_empty_product_clone_recipes",
        group: "positive",
        source_sha256: "0f0165a502b8a5ae64e736c3a2a67fcf5ebeab6bc7432da44145f8be6f0ae09f",
        full_source: r###"fn run()->i32{let n=10i32.clone();let yes=true.clone();let unit=().clone();let empty:[i32;0]=[];let cloned=empty.clone();let unit_again=unit.clone();if yes{n}else{0}}
fn main(){if run()==10{println!("canonical copy oracle ok");}}
"###,
        expected_stdout: "canonical copy oracle ok\n",
    },
    Fixture {
        id: "P07",
        name: "manual_clone_on_copy_nominal_preserves_observable_behavior",
        group: "positive",
        source_sha256: "209439262ba12a3c35b7b0583f142e5cda10c0de58d6b1d2f6b18a922b66f01b",
        full_source: r###"struct S{v:i32}impl Clone for S{fn clone(&self)->Self{S{v:self.v+5}}}impl Copy for S{}fn run()->i32{let s=S{v:1};let cloned=s.clone();let before=s.v;let after=cloned.v;before*100+after}
fn main(){if run()==106{println!("canonical copy oracle ok");}}
"###,
        expected_stdout: "canonical copy oracle ok\n",
    },
    Fixture {
        id: "P08",
        name: "tuple_clone_preserves_nontrivial_manual_copy_child_clone",
        group: "positive",
        source_sha256: "548669d0402a3219237124137de3a861ab0fe03fd1b11219178d35061f4d97f4",
        full_source: r###"struct S{v:i32}impl Clone for S{fn clone(&self)->Self{S{v:self.v+1}}}impl Copy for S{}fn run()->i32{let pair=(S{v:1},S{v:2});let cloned=pair.clone();let first=cloned.0.v;let second=cloned.1.v;first*100+second}
fn main(){if run()==203{println!("canonical copy oracle ok");}}
"###,
        expected_stdout: "canonical copy oracle ok\n",
    },
    Fixture {
        id: "P09",
        name: "array_clone_preserves_nontrivial_manual_copy_child_clone",
        group: "positive",
        source_sha256: "d2754623697ca10bfe2cbca425e0a91527c809fe396a758bc855135d5bd04863",
        full_source: r###"struct S{v:i32}impl Clone for S{fn clone(&self)->Self{S{v:self.v+1}}}impl Copy for S{}fn run()->i32{let array=[S{v:3},S{v:4}];let cloned=array.clone();let first=cloned[0].v;let second=cloned[1].v;first*100+second}
fn main(){if run()==405{println!("canonical copy oracle ok");}}
"###,
        expected_stdout: "canonical copy oracle ok\n",
    },
    Fixture {
        id: "P10",
        name: "shared_reference_clone_inner_escapes_outer_handle",
        group: "positive",
        source_sha256: "8f0799321aea9e3d74fba916565a7e4fb8c834c28079f8db6075153ded4c3661",
        full_source: r###"struct Opaque{v:i32}fn run()->i32{let owner=Opaque{v:11};let result;{let r=&owner;result=<&Opaque as core::clone::Clone>::clone(&r);}result.v}
fn main(){if run()==11{println!("canonical copy oracle ok");}}
"###,
        expected_stdout: "canonical copy oracle ok\n",
    },
    Fixture {
        id: "P11",
        name: "receiver_candidate_pointee_clone_precedes_reference_clone",
        group: "positive",
        source_sha256: "f67fefdc658e798a78e0e00476976695bad558d104476f7a28cd0e9030369979",
        full_source: r###"struct S{v:i32}impl Clone for S{fn clone(&self)->Self{S{v:self.v+1}}}fn run()->i32{let s=S{v:12};let r=&s;let cloned=r.clone();let original=s.v;let value=cloned.v;original*100+value}
fn main(){if run()==1213{println!("canonical copy oracle ok");}}
"###,
        expected_stdout: "canonical copy oracle ok\n",
    },
    Fixture {
        id: "P12",
        name: "receiver_candidate_shared_reference_fallback_and_inherent_priority",
        group: "positive",
        source_sha256: "d83fbcce37ccc17cded20bc6aa3f376f7aeec1c99d4d38a936e63d0b4699b7cb",
        full_source: r###"struct Opaque{v:i32}struct S{v:i32}impl S{fn clone(&self)->i32{self.v+10}}impl Clone for S{fn clone(&self)->Self{S{v:self.v+100}}}fn run()->i32{let owner=Opaque{v:13};let r=&owner;let cloned=r.clone();let first=cloned.v;let s=S{v:2};let second=s.clone();first*100+second}
fn main(){if run()==1312{println!("canonical copy oracle ok");}}
"###,
        expected_stdout: "canonical copy oracle ok\n",
    },
    Fixture {
        id: "P13",
        name: "real_clone_from_override_runs_while_default_stays_unavailable",
        group: "positive",
        source_sha256: "f7ac913d8155dc2c4dd267be3ed5ae5c63d9ddf124f0f4e4befbadd74f61364f",
        full_source: r###"struct S{v:i32}impl Clone for S{fn clone(&self)->Self{S{v:self.v}}fn clone_from(&mut self,source:&Self){self.v=source.v+10;}}fn run()->i32{let mut destination=S{v:1};let source=S{v:2};destination.clone_from(&source);destination.v}
fn main(){if run()==12{println!("canonical copy oracle ok");}}
"###,
        expected_stdout: "canonical copy oracle ok\n",
    },
    Fixture {
        id: "P14",
        name: "logical_copy_clone_double_shared_reference_without_method_execution",
        group: "positive",
        source_sha256: "1a3571936a1faca776ce8b9f4ae45c6c3ffd3c049185d12ad3e6332476ccd1df",
        full_source: r###"fn run()->i32{let v=7;let r=&v;let rr=&r;let a=rr;let b=rr;**a+**b}
fn main(){if run()==14{println!("canonical copy oracle ok");}}
"###,
        expected_stdout: "canonical copy oracle ok\n",
    },
    Fixture {
        id: "P15",
        name: "T_U_coincidence_preserves_declared_clone_result_origins",
        group: "positive",
        source_sha256: "8c39ac46818ced73cbe066bfb4b84bb71f4336da43d6e0891f5e8cb7e4c29b0e",
        full_source: r###"fn keep<'a,'b,T,U>(x:&'a T,y:&'b U)->&'a T{let ignored=y;<&T as core::clone::Clone>::clone(&x)}fn run()->i32{let long=17;let out;{let short=2;out=keep::<i32,i32>(&long,&short);}*out}
fn main(){if run()==17{println!("canonical copy oracle ok");}}
"###,
        expected_stdout: "canonical copy oracle ok\n",
    },
    Fixture {
        id: "P16",
        name: "unused_nominal_field_tuple33_array4096_does_not_poison_scalar_main",
        group: "positive",
        source_sha256: "f244a6bc638af5146c72fe8986162d359880db91064f8ceeb8f54ab2409fb5cf",
        full_source: r###"struct UnusedLarge{tuple:(i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32),array:[i32;4096]}fn run()->i32{19}
fn main(){if run()==19{println!("canonical copy oracle ok");}}
"###,
        expected_stdout: "canonical copy oracle ok\n",
    },
    Fixture {
        id: "N17",
        name: "fake_copy_spelling_does_not_grant_second_owned_use",
        group: "invalid",
        source_sha256: "2ddf3302dbb4432781e9d28bd4a3a0aef1792875444cc0d4d9d09c2ccfd0c550",
        full_source: r###"trait Copy{fn tag(&self)->i32;}struct S{v:i32}impl Copy for S{fn tag(&self)->i32{self.v}}fn run()->i32{let s=S{v:1};let a=s;let b=s;a.v+b.v}
fn main(){if run()==0{println!("canonical copy oracle ok");}}
"###,
        expected_stdout: "",
    },
    Fixture {
        id: "N18",
        name: "copy_shaped_nominal_without_copy_impl_remains_move_only",
        group: "invalid",
        source_sha256: "bc5481638576b986847ed2da94f52ae7d7402b702b134eb0bee2ac9f97e652ca",
        full_source: r###"struct S{v:i32}fn run()->i32{let s=S{v:1};let a=s;let b=s;a.v+b.v}
fn main(){if run()==0{println!("canonical copy oracle ok");}}
"###,
        expected_stdout: "",
    },
    Fixture {
        id: "N19",
        name: "conditional_copy_impl_missing_required_field_bound",
        group: "invalid",
        source_sha256: "6ec796b8f063a3b2bdb60f8e97076eed69f71b46f4a2caa1ca7f2f962634c61e",
        full_source: r###"struct S<T>{v:T}impl<T:Clone> Clone for S<T>{fn clone(&self)->Self{S{v:self.v.clone()}}}impl<T:Clone> Copy for S<T>{}fn run()->i32{0}
fn main(){if run()==0{println!("canonical copy oracle ok");}}
"###,
        expected_stdout: "",
    },
    Fixture {
        id: "N20",
        name: "copy_impl_without_clone_supertrait_is_invalid",
        group: "invalid",
        source_sha256: "5d2a58855f0db241a49f880d6b9be31b839b4bc0003d90311b4d69ab621e2495",
        full_source: r###"struct S{v:i32}impl Copy for S{}fn run()->i32{0}
fn main(){if run()==0{println!("canonical copy oracle ok");}}
"###,
        expected_stdout: "",
    },
    Fixture {
        id: "N21",
        name: "unused_bad_clone_body_moves_noncopy_through_shared_self",
        group: "invalid",
        source_sha256: "b56a995752409d1a6d99efe116b0996569e897d53b9d0a009e8d590bdb581e8f",
        full_source: r###"struct S{v:i32}impl Clone for S{fn clone(&self)->Self{*self}}fn run()->i32{0}
fn main(){if run()==0{println!("canonical copy oracle ok");}}
"###,
        expected_stdout: "",
    },
    Fixture {
        id: "N22",
        name: "canonical_copy_with_drop_is_invalid_before_unsupported",
        group: "invalid",
        source_sha256: "28105d1725a6aa878e2bc3666fd68856c5b8a76fc136c36553c874cf0f1ef355",
        full_source: r###"struct S{v:i32}impl Clone for S{fn clone(&self)->Self{S{v:self.v}}}impl Copy for S{}impl Drop for S{fn drop(&mut self){}}fn run()->i32{0}
fn main(){if run()==0{println!("canonical copy oracle ok");}}
"###,
        expected_stdout: "",
    },
    Fixture {
        id: "N23",
        name: "foreign_canonical_copy_or_clone_impl_is_orphan_invalid",
        group: "invalid",
        source_sha256: "7cfa819e670da64a78167196366837b4da372ca08303da4fcd780277dc2b808e",
        full_source: r###"impl core::clone::Clone for i32{fn clone(&self)->Self{*self}}fn run()->i32{0}
fn main(){if run()==0{println!("canonical copy oracle ok");}}
"###,
        expected_stdout: "",
    },
    Fixture {
        id: "N24",
        name: "explicit_canonical_sized_impl_is_invalid",
        group: "invalid",
        source_sha256: "bc21b6b4909f3307d01c90ae50b871f3c78fa632bf4daa2e1e6794034ed12c89",
        full_source: r###"struct S{v:i32}impl core::marker::Sized for S{}fn run()->i32{0}
fn main(){if run()==0{println!("canonical copy oracle ok");}}
"###,
        expected_stdout: "",
    },
    Fixture {
        id: "N25",
        name: "mutable_reference_does_not_gain_copy_permission",
        group: "invalid",
        source_sha256: "0c7fe47c6a129cfbbf9e3449e48db8ef0aa2c9b5b0144b5614df53d5cb04e655",
        full_source: r###"fn run()->i32{let mut v=1;let r=&mut v;let a=r;let b=r;*a+*b}
fn main(){if run()==0{println!("canonical copy oracle ok");}}
"###,
        expected_stdout: "",
    },
    Fixture {
        id: "N26",
        name: "shared_reference_clone_result_escapes_original_referent",
        group: "invalid",
        source_sha256: "74f5dafc034d219be57d291c6398e417ad1e9e611a1d366cd797c4ad0ceba248",
        full_source: r###"fn run()->i32{let out;{let v=1;let r=&v;out=<&i32 as core::clone::Clone>::clone(&r);}*out}
fn main(){if run()==0{println!("canonical copy oracle ok");}}
"###,
        expected_stdout: "",
    },
    Fixture {
        id: "N27",
        name: "wrong_T_U_clone_result_region_despite_concrete_coincidence",
        group: "invalid",
        source_sha256: "0eaa1a42ef48fb11b825dc2c2ddd31213cfd0c87c2940f95356ccee287aac52a",
        full_source: r###"fn wrong<'a,'b,T,U>(x:&'a T,y:&'b U)->&'a U{let ignored=x;<&U as core::clone::Clone>::clone(&y)}fn run()->i32{let long=1;let out;{let short=2;out=wrong::<i32,i32>(&long,&short);}*out}
fn main(){if run()==0{println!("canonical copy oracle ok");}}
"###,
        expected_stdout: "",
    },
    Fixture {
        id: "N28",
        name: "zero_length_array_field_still_requires_element_copy",
        group: "invalid",
        source_sha256: "bf412fe1134898af32e228772cc2cc1bd137f0450270c80a216570c38dbf2cbd",
        full_source: r###"struct S<T>{v:[T;0]}impl<T:Clone> Clone for S<T>{fn clone(&self)->Self{S{v:self.v.clone()}}}impl<T:Clone> Copy for S<T>{}fn run()->i32{0}
fn main(){if run()==0{println!("canonical copy oracle ok");}}
"###,
        expected_stdout: "",
    },
    Fixture {
        id: "X29",
        name: "ordinary_reference_generic_actual_clone_wrapper",
        group: "legal-excluded",
        source_sha256: "118565f4ced1cbbf881122ba2800c068ed02f413548f0c7f6bf3872c98fe460d",
        full_source: r###"fn wrap<T:Clone>(x:T)->T{x.clone()}fn run()->i32{let v=20;let r=wrap::<&i32>(&v);*r}
fn main(){if run()==20{println!("canonical copy oracle ok");}}
"###,
        expected_stdout: "canonical copy oracle ok\n",
    },
    Fixture {
        id: "X30",
        name: "unavailable_canonical_clone_from_default_demand",
        group: "legal-excluded",
        source_sha256: "ddbd708f697cae3e57af0dbce6312486730dcfd6375910c81bcd4d9af1bf65ae",
        full_source: r###"struct S{v:i32}impl Clone for S{fn clone(&self)->Self{S{v:self.v}}}fn run()->i32{let mut destination=S{v:1};let source=S{v:2};destination.clone_from(&source);destination.v}
fn main(){if run()==2{println!("canonical copy oracle ok");}}
"###,
        expected_stdout: "canonical copy oracle ok\n",
    },
    Fixture {
        id: "X31",
        name: "forced_clone_on_double_shared_reference_requires_depth3_receiver",
        group: "legal-excluded",
        source_sha256: "68e4bce9ad409caf4dd2064e5dbdc2be62208d59394ddc1f7c4403ca6aaa34a8",
        full_source: r###"fn run()->i32{let v=22;let r=&v;let rr=&r;let cloned=<&&i32 as core::clone::Clone>::clone(&rr);**cloned}
fn main(){if run()==22{println!("canonical copy oracle ok");}}
"###,
        expected_stdout: "canonical copy oracle ok\n",
    },
    Fixture {
        id: "X32",
        name: "legal_canonical_drop_only_requires_destructor_execution",
        group: "legal-excluded",
        source_sha256: "28fc4d7000d2464e5f7f11cfb3467592099b3de0b6e8da40432963667b7e5de6",
        full_source: r###"struct S{v:i32}impl Drop for S{fn drop(&mut self){self.v=0;}}fn run()->i32{let s=S{v:23};let result=s.v;result}
fn main(){if run()==23{println!("canonical copy oracle ok");}}
"###,
        expected_stdout: "canonical copy oracle ok\n",
    },
];

fn group(group: &str, expected_count: usize) -> Vec<&'static Fixture> {
    assert_eq!(FIXTURES.len(), 32);
    let selected: Vec<_> = FIXTURES
        .iter()
        .filter(|fixture| fixture.group == group)
        .collect();
    assert_eq!(selected.len(), expected_count);
    for fixture in &selected {
        assert!(!fixture.id.is_empty() && !fixture.name.is_empty());
        assert_eq!(fixture.source_sha256.len(), 64);
        assert!(
            fixture
                .source_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        );
    }
    selected
}

#[test]
fn frozen_canonical_copy_positive16() {
    for fixture in group("positive", 16) {
        runs(fixture.full_source, fixture.expected_stdout);
    }
}

#[test]
fn frozen_canonical_copy_invalid12() {
    for fixture in group("invalid", 12) {
        rejects(fixture.name, fixture.full_source, true);
    }
}

#[test]
fn frozen_canonical_copy_legal_excluded4() {
    for fixture in group("legal-excluded", 4) {
        // Keep the historical group and every other unsupported boundary unchanged.
        // X32 is promoted by DROP-NORMAL.md: its unchanged source is also the
        // frozen drop_normal positive/canonical_copy_x32.rs stock-matched case.
        if matches!(
            fixture.name,
            "ordinary_reference_generic_actual_clone_wrapper"
                | "forced_clone_on_double_shared_reference_requires_depth3_receiver"
                | "legal_canonical_drop_only_requires_destructor_execution"
        ) {
            runs(fixture.full_source, fixture.expected_stdout);
        } else {
            rejects(fixture.name, fixture.full_source, false);
        }
    }
}
