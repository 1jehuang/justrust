//! Frozen exact32 executable generic carrier acceptance corpus.
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
                "jr-generic-carriers-{}-{epoch}-{id}",
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
// Exact observed corpus SHA256: 400bf44235e1c5a34ff964a10ab61111696d7765010a3e66bd1d1bcf602f8842
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
        name: "explicit_shared_root_actual_identity",
        group: "positive",
        source_sha256: "11ca46a9a72fb38fe0baf6680e9bdac677a41bbc6e0e796dfac4158ae45b05d1",
        full_source: r###"fn id<T>(x:T)->T{x}fn run()->i32{let v=1;let r=id::<&i32>(&v);*r}
fn main(){if run()==1{println!("generic carriers oracle ok");}}
"###,
        expected_stdout: "generic carriers oracle ok\n",
    },
    Fixture {
        id: "P02",
        name: "rigid_parameter_forwarding_preserves_hidden_origins",
        group: "positive",
        source_sha256: "dba108df3aee00a92c64d335d85c6c8d4cd3253c7d540d358499b132302c96dc",
        full_source: r###"fn id<T>(x:T)->T{x}fn forward<T>(x:T)->T{id::<T>(x)}fn run()->i32{let v=2;let r=forward::<&i32>(&v);*r}
fn main(){if run()==2{println!("generic carriers oracle ok");}}
"###,
        expected_stdout: "generic carriers oracle ok\n",
    },
    Fixture {
        id: "P03",
        name: "shared_root_copy_without_referent_copy",
        group: "positive",
        source_sha256: "aed81ad7d352b36963db15884f0597d3588de6ad2c08ef7c610affd60d49a17b",
        full_source: r###"struct S{v:i32}fn twice<T:Copy>(x:T)->(T,T){(x,x)}fn run()->i32{let s=S{v:3};let pair=twice::<&S>(&s);let a=pair.0.v;let b=pair.1.v;a+b}
fn main(){if run()==6{println!("generic carriers oracle ok");}}
"###,
        expected_stdout: "generic carriers oracle ok\n",
    },
    Fixture {
        id: "P04",
        name: "single_affine_mutable_root_identity_transfer",
        group: "positive",
        source_sha256: "aae1dcac11b67cbfb065dd9ed01558d89bf56b3650fc3ffc5f7c4f883a47cfee",
        full_source: r###"fn id<T>(x:T)->T{x}fn run()->i32{let mut v=0;let r=id::<&mut i32>(&mut v);*r=4;v}
fn main(){if run()==4{println!("generic carriers oracle ok");}}
"###,
        expected_stdout: "generic carriers oracle ok\n",
    },
    Fixture {
        id: "P05",
        name: "internal_tuple_components_keep_distinct_occurrence_origins",
        group: "positive",
        source_sha256: "d865f03f5c05c47e1857491512644e5863a070c1e6b489cfc62c7c3d58493bd6",
        full_source: r###"fn pair<T>(x:T,y:T)->(T,T){(x,y)}fn run()->i32{let a=5;let b=6;let p=pair::<&i32>(&a,&b);let first=*p.0;let second=*p.1;first*100+second}
fn main(){if run()==506{println!("generic carriers oracle ok");}}
"###,
        expected_stdout: "generic carriers oracle ok\n",
    },
    Fixture {
        id: "P06",
        name: "internal_array_constant_index_component_transport",
        group: "positive",
        source_sha256: "2c428590d8ca610bcd66fb2c72405b2039cc604ea7a111fbd20ad69f19f73f89",
        full_source: r###"fn pair<T>(x:T,y:T)->[T;2]{[x,y]}fn run()->i32{let a=6;let b=7;let p=pair::<&i32>(&a,&b);let first=*p[0];let second=*p[1];first*100+second}
fn main(){if run()==607{println!("generic carriers oracle ok");}}
"###,
        expected_stdout: "generic carriers oracle ok\n",
    },
    Fixture {
        id: "P07",
        name: "internal_nominal_variant_component_correspondence",
        group: "positive",
        source_sha256: "73cfe29f0262d4cbec5f897dc96118d88bdd4a07f32cdea4bceade061441a530",
        full_source: r###"struct Wrap<T>{value:T}enum E<T>{One{value:T}}fn wrap<T>(x:T)->E<T>{let w=Wrap{value:x};E::One{value:w.value}}fn run()->i32{let v=7;let e=wrap::<&i32>(&v);match e{E::One{value}=>*value}}
fn main(){if run()==7{println!("generic carriers oracle ok");}}
"###,
        expected_stdout: "generic carriers oracle ok\n",
    },
    Fixture {
        id: "P08",
        name: "partial_component_move_preserves_sibling_carriers",
        group: "positive",
        source_sha256: "fca6e0bbea323d5491c795f04fe0712a06d43060727db4a104fa7fa6db51efa4",
        full_source: r###"fn second<T>(x:T,y:T)->T{let pair=(x,y);let first=pair.0;let second=pair.1;let _=first;second}fn run()->i32{let a=1;let b=8;let r=second::<&i32>(&a,&b);*r}
fn main(){if run()==8{println!("generic carriers oracle ok");}}
"###,
        expected_stdout: "generic carriers oracle ok\n",
    },
    Fixture {
        id: "P09",
        name: "shared_carrier_put_replacement_preserves_incoming_origin",
        group: "positive",
        source_sha256: "bdd0dbdb91d1041574987166f4ef95e764e69e381725c6c74db648ced4d6e5b7",
        full_source: r###"fn put<T>(dst:&mut T,x:T){*dst=x;}fn run()->i32{let a=1;let b=9;let mut slot=&a;put::<&i32>(&mut slot,&b);*slot}
fn main(){if run()==9{println!("generic carriers oracle ok");}}
"###,
        expected_stdout: "generic carriers oracle ok\n",
    },
    Fixture {
        id: "P10",
        name: "single_affine_put_preserves_checked_child_dependencies",
        group: "positive",
        source_sha256: "8cd777dda29fa03cea1448c6fe53e6c2bb172bdd67dcaf491e8af2e8e72abb3d",
        full_source: r###"fn put<T>(dst:&mut T,x:T){*dst=x;}fn run()->i32{let mut a=1;let mut b=2;let mut slot=&mut a;put::<&mut i32>(&mut slot,&mut b);*slot=10;b}
fn main(){if run()==10{println!("generic carriers oracle ok");}}
"###,
        expected_stdout: "generic carriers oracle ok\n",
    },
    Fixture {
        id: "P11",
        name: "current_cell_reload_and_return_follow_written_poststate",
        group: "positive",
        source_sha256: "9f2b3838b1b7df3d50e43f7843c79ba70e1302b4bc8d8ee0fea26bc6cc570058",
        full_source: r###"fn replace<T:Copy>(dst:&mut T,x:T)->T{*dst=x;*dst}fn run()->i32{let a=1;let b=11;let mut slot=&a;let out=replace::<&i32>(&mut slot,&b);let first=*out;let second=*slot;first*100+second}
fn main(){if run()==1111{println!("generic carriers oracle ok");}}
"###,
        expected_stdout: "generic carriers oracle ok\n",
    },
    Fixture {
        id: "P12",
        name: "derived_aliases_observe_one_current_canonical_cell",
        group: "positive",
        source_sha256: "7fa59032bc00355e3ddaae27676b19f830ce7a59e3ea6a0c297adbb43c167b25",
        full_source: r###"fn put<T>(dst:&mut T,x:T){*dst=x;}fn run()->i32{let a=1;let b=12;let mut slot=&a;let parent=&mut slot;{let child=&mut *parent;put::<&i32>(child,&b);}**parent}
fn main(){if run()==12{println!("generic carriers oracle ok");}}
"###,
        expected_stdout: "generic carriers oracle ok\n",
    },
    Fixture {
        id: "P13",
        name: "separate_call_occurrences_keep_fresh_lifetime_bindings",
        group: "positive",
        source_sha256: "97a6ec0711fa295012d3385e906e79b3e8871b77a2c1ab90ba71d050d1f36d8f",
        full_source: r###"fn id<T>(x:T)->T{x}fn run()->i32{let long=13;let a=id::<&i32>(&long);let first=*a;let second;{let short=14;let b=id::<&i32>(&short);second=*b;}first*100+second}
fn main(){if run()==1314{println!("generic carriers oracle ok");}}
"###,
        expected_stdout: "generic carriers oracle ok\n",
    },
    Fixture {
        id: "P14",
        name: "shared_join_and_rebind_union_component_origins",
        group: "positive",
        source_sha256: "6f2ec2a3ab885769b8e7bea4ef32849618a10db3b88f405d030ce9e1cc2d62a3",
        full_source: r###"fn choose<T>(flag:bool,x:T,y:T)->T{if flag{x}else{y}}fn run()->i32{let a=14;let b=15;let mut r=choose::<&i32>(true,&a,&b);let first=*r;r=choose::<&i32>(false,&a,&b);let second=*r;first*100+second}
fn main(){if run()==1415{println!("generic carriers oracle ok");}}
"###,
        expected_stdout: "generic carriers oracle ok\n",
    },
    Fixture {
        id: "P15",
        name: "firm_explicit_mutable_expectation_retains_checked_reborrow",
        group: "positive",
        source_sha256: "7f85f3fae91a705c205567bb33829d89ebb1be7503b8f5c7a93ed24730558512",
        full_source: r###"fn id<T>(x:T)->T{x}fn run()->i32{let mut v=1;let r=&mut v;{let child=id::<&mut i32>(r);*child=14;}*r=15;*r}
fn main(){if run()==15{println!("generic carriers oracle ok");}}
"###,
        expected_stdout: "generic carriers oracle ok\n",
    },
    Fixture {
        id: "P16",
        name: "recursive_shared_forwarding_uses_declared_envelope",
        group: "positive",
        source_sha256: "1def6256fc441d8956819516d368518894e6068cc1bb09c1cf435f594f000bd6",
        full_source: r###"fn walk<T:Copy>(x:T,n:i32)->T{if n==0{x}else{walk::<T>(x,n-1)}}fn run()->i32{let v=16;let r=walk::<&i32>(&v,3);*r}
fn main(){if run()==16{println!("generic carriers oracle ok");}}
"###,
        expected_stdout: "generic carriers oracle ok\n",
    },
    Fixture {
        id: "N17",
        name: "local_referent_escapes_through_opaque_return",
        group: "invalid",
        source_sha256: "acbde04f31a3f5f6e081c2b631d06c24ceb2004536a1082591a616a7c9de73cb",
        full_source: r###"fn id<T>(x:T)->T{x}fn run()->i32{let out;{let local=17;out=id::<&i32>(&local);}*out}
fn main(){if run()==0{println!("generic carriers oracle ok");}}
"###,
        expected_stdout: "",
    },
    Fixture {
        id: "N18",
        name: "short_incoming_reference_escapes_external_put_cell",
        group: "invalid",
        source_sha256: "e706f41243d3f45ac8ff24b3099fc63cbea851460cd975253b8d7a43ea30ebe9",
        full_source: r###"fn put<T>(dst:&mut T,x:T){*dst=x;}fn run()->i32{let long=1;let mut slot=&long;{let short=18;put::<&i32>(&mut slot,&short);}*slot}
fn main(){if run()==0{println!("generic carriers oracle ok");}}
"###,
        expected_stdout: "",
    },
    Fixture {
        id: "N19",
        name: "mutable_hidden_component_region_invariance_violation",
        group: "invalid",
        source_sha256: "0421f444bf2e109fd2aeb8f66a0c82444ba2c726088a17ed3852889ef08b8c0d",
        full_source: r###"fn put<T>(dst:&mut T,x:T){*dst=x;}fn invariant<'a,'b>(dst:&mut &'a i32,x:&'b i32){put::<&'a i32>(dst,x);}fn run()->i32{0}
fn main(){if run()==0{println!("generic carriers oracle ok");}}
"###,
        expected_stdout: "",
    },
    Fixture {
        id: "N20",
        name: "same_erased_type_reuses_unrelated_call_lifetime",
        group: "invalid",
        source_sha256: "fe40edb6837538af73a9fc6ddbe4e57f56cb742ad80be7b7631664552c03277a",
        full_source: r###"fn id<T>(x:T)->T{x}fn run()->i32{let long=1;let out;{let short=20;let a=id::<&i32>(&long);let b=id::<&i32>(&short);out=b;let _=a;}*out}
fn main(){if run()==0{println!("generic carriers oracle ok");}}
"###,
        expected_stdout: "",
    },
    Fixture {
        id: "N21",
        name: "distinct_parameter_occurrences_cannot_exchange_return_authority",
        group: "invalid",
        source_sha256: "480469fd43e8b5b94398016f54e5e8edd8450a07d8601c467d69ea48d7cbd197",
        full_source: r###"fn id<T>(x:T)->T{x}fn wrong<'a,'b,T,U>(x:&'a T,y:&'b U)->&'a U{let _=x;id::<&'b U>(y)}fn run()->i32{let long=1;let out;{let short=21;out=wrong::<i32,i32>(&long,&short);}*out}
fn main(){if run()==0{println!("generic carriers oracle ok");}}
"###,
        expected_stdout: "",
    },
    Fixture {
        id: "N22",
        name: "affine_root_actual_is_used_after_explicit_move",
        group: "invalid",
        source_sha256: "314e848a480bb77a027ea6a90911a7c731885e8a8c4754ba4e9dc5fd15e9d3e1",
        full_source: r###"fn id<T>(x:T)->T{x}fn bad<T>(x:T)->T{let first=id::<T>(x);let second=id::<T>(x);let _=first;second}fn run()->i32{let mut v=22;let r=bad::<&mut i32>(&mut v);*r}
fn main(){if run()==0{println!("generic carriers oracle ok");}}
"###,
        expected_stdout: "",
    },
    Fixture {
        id: "N23",
        name: "unbounded_opaque_parameter_is_duplicated_without_copy",
        group: "invalid",
        source_sha256: "325eb64ac059f5d9a29567449409bdf1002713802d4beed5dfe1653fa932492a",
        full_source: r###"fn duplicate<T>(x:T)->(T,T){(x,x)}fn run()->i32{let v=23;let p=duplicate::<&i32>(&v);*p.0+*p.1}
fn main(){if run()==0{println!("generic carriers oracle ok");}}
"###,
        expected_stdout: "",
    },
    Fixture {
        id: "N24",
        name: "indirect_put_through_shared_handle_is_invalid",
        group: "invalid",
        source_sha256: "fd3e7a837e33b610b6e11848d1ff0f8001a0ff8805a804ac666b3eedaf27cddc",
        full_source: r###"fn wrong<T>(dst:&T,x:T){*dst=x;}fn run()->i32{let v=24;let slot=&v;wrong::<&i32>(&slot,&v);0}
fn main(){if run()==0{println!("generic carriers oracle ok");}}
"###,
        expected_stdout: "",
    },
    Fixture {
        id: "N25",
        name: "live_derived_shared_alias_conflicts_with_mutable_cell_access",
        group: "invalid",
        source_sha256: "0b74882b57f18ca5f46f5d9ca14bddac42926cca0581bb2fb44ada2cf5ab2cd3",
        full_source: r###"fn put<T>(dst:&mut T,x:T){*dst=x;}fn run()->i32{let a=1;let b=25;let mut slot=&a;let parent=&mut slot;let alias=&*parent;put::<&i32>(parent,&b);**alias}
fn main(){if run()==0{println!("generic carriers oracle ok");}}
"###,
        expected_stdout: "",
    },
    Fixture {
        id: "N26",
        name: "stored_reborrow_child_does_not_reactivate_parent",
        group: "invalid",
        source_sha256: "95ec51aac2a0dfa4ef7cbcc5295cabb2d0c50f7b9d19c3da81f2c6fac11cd102",
        full_source: r###"fn bad<'a,'b>(p:&'a mut &'b mut i32)->&'b mut i32{&mut **p}fn run()->i32{0}
fn main(){if run()==0{println!("generic carriers oracle ok");}}
"###,
        expected_stdout: "",
    },
    Fixture {
        id: "N27",
        name: "returned_written_cell_cannot_hide_short_incoming_origin",
        group: "invalid",
        source_sha256: "d38b32d26ec3d443ccc303390201457d10ed6656cbb7409c2d65dfc8431fe3ee",
        full_source: r###"fn replace<T:Copy>(dst:&mut T,x:T)->T{*dst=x;*dst}fn run()->i32{let long=1;let mut slot=&long;let out;{let short=27;out=replace::<&i32>(&mut slot,&short);}*out}
fn main(){if run()==0{println!("generic carriers oracle ok");}}
"###,
        expected_stdout: "",
    },
    Fixture {
        id: "N28",
        name: "unused_generic_body_has_invalid_carrier_escape",
        group: "invalid",
        source_sha256: "2199bb237fcffb264ff98399a3b7cf88596962e39725fdbb2cc0b76015a32555",
        full_source: r###"fn bad<'a,T>(x:T)->&'a i32{let local=28;let _=x;&local}fn run()->i32{0}
fn main(){if run()==0{println!("generic carriers oracle ok");}}
"###,
        expected_stdout: "",
    },
    Fixture {
        id: "X29",
        name: "newly_inferred_structural_reference_actual",
        group: "legal-excluded",
        source_sha256: "d3cb19f3247860db9317cdd9a019b63c7c076dae483faf56440a0ba1c39694e4",
        full_source: r###"fn id<T>(x:T)->T{x}fn run()->i32{let v=29;let out=id(&v);*out}
fn main(){if run()==29{println!("generic carriers oracle ok");}}
"###,
        expected_stdout: "generic carriers oracle ok\n",
    },
    Fixture {
        id: "X30",
        name: "multi_expanded_affine_result_components",
        group: "legal-excluded",
        source_sha256: "1c3b169441a3f2bb96f9d0887782c51c7e74843f0f2d4fc4b37df235ea1e67ff",
        full_source: r###"fn pair<T,U>(x:T,y:U)->(T,U){(x,y)}fn run()->i32{let mut a=1;let mut b=2;let (x,y)=pair::<&mut i32,&mut i32>(&mut a,&mut b);*x=10;*y=20;a+b}
fn main(){if run()==30{println!("generic carriers oracle ok");}}
"###,
        expected_stdout: "generic carriers oracle ok\n",
    },
    Fixture {
        id: "X31",
        name: "mixed_affine_result_and_written_cell_flows",
        group: "legal-excluded",
        source_sha256: "e156d4741f4792417dee17a59fe195cf672377d512dfa63ee49e1de2933f663b",
        full_source: r###"fn put_and_view<'a,T>(dst:&'a mut T,x:T)->&'a mut T{*dst=x;dst}fn run()->i32{let mut a=1;let mut b=2;let mut slot=&mut a;let r=put_and_view::<&mut i32>(&mut slot,&mut b);**r=31;b}
fn main(){if run()==31{println!("generic carriers oracle ok");}}
"###,
        expected_stdout: "generic carriers oracle ok\n",
    },
    Fixture {
        id: "X32",
        name: "derived_internal_nominal_reference_actual_call",
        group: "positive",
        source_sha256: "09e40d1c8632afb2dcc97e16c95332fe154d57327eb5822069fe3971afe40ae1",
        full_source: r###"struct Wrap<T>{value:T}fn id<T>(x:T)->T{x}fn outer<T>(x:T)->T{let wrapped=Wrap{value:x};let out=id::<Wrap<T>>(wrapped);out.value}fn run()->i32{let v=32;let r=outer::<&i32>(&v);*r}
fn main(){if run()==32{println!("generic carriers oracle ok");}}
"###,
        expected_stdout: "generic carriers oracle ok\n",
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
fn frozen_generic_carriers_positive17() {
    for fixture in group("positive", 17) {
        runs(fixture.full_source, fixture.expected_stdout);
    }
}

#[test]
fn frozen_generic_carriers_invalid12() {
    for fixture in group("invalid", 12) {
        rejects(fixture.name, fixture.full_source, true);
    }
}

#[test]
fn frozen_generic_carriers_legal_excluded3() {
    for fixture in group("legal-excluded", 3) {
        rejects(fixture.name, fixture.full_source, false);
    }
}
