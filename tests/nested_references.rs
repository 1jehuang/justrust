//! Frozen exact32 bounded two-layer shared-reference acceptance corpus.
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
                "jr-nested-references-{}-{epoch}-{id}",
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
// Exact observed corpus SHA256: 0554a1035124644fb649691423b3706888c4aa6395437f92c493c298a6b49432
// The Python companion retains every dictionary field verbatim.
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
        name: "nested_shared_identity_preserves_both_carrier_layers",
        group: "positive",
        source_sha256: "7da374e27c1648ec784ff59a48e619a397f553633025f0773d3b2a7fd24b2506",
        full_source: r###"fn id<'o,'i,T>(x:&'o &'i T)->&'o &'i T{x}
fn run()->i32{let v=11;let r=&v;let rr=id(&r);let oracle_observation0=**rr;oracle_observation0}
fn main(){if run()==11{println!("nested references oracle ok");}}
"###,
        expected_stdout: "nested references oracle ok\n",
    },
    Fixture {
        id: "P02",
        name: "load_inner_transports_original_referent_origins",
        group: "positive",
        source_sha256: "a61470a9de1e39615e6a4e9feeed47cf09b48c3668e6ebdca4cdca63226e78bd",
        full_source: r###"fn inner<'o,'i,T>(x:&'o &'i T)->&'i T{*x}
fn run()->i32{let v=12;let r=&v;let x=inner(&r);let oracle_observation0=*x;oracle_observation0}
fn main(){if run()==12{println!("nested references oracle ok");}}
"###,
        expected_stdout: "nested references oracle ok\n",
    },
    Fixture {
        id: "P03",
        name: "loaded_inner_escapes_temporary_outer_handle_scope",
        group: "positive",
        source_sha256: "53f76a48527c310f5721b3790b7df6c58c81551867f0b97058583ce7a62b309b",
        full_source: r###"fn inner<'o,'i,T>(x:&'o &'i T)->&'i T{*x}
fn run()->i32{let v=13;let r=&v;let out;{let rr=&r;out=inner(rr);}let oracle_observation0=*out;oracle_observation0}
fn main(){if run()==13{println!("nested references oracle ok");}}
"###,
        expected_stdout: "nested references oracle ok\n",
    },
    Fixture {
        id: "P04",
        name: "loaded_inner_escapes_local_inner_binding_scope",
        group: "positive",
        source_sha256: "f7fe1464580a34a6632a2d37dd51a4718c2cc92e80c3e98328857625fe0522f8",
        full_source: r###"fn run()->i32{let v=14;let out;{let r=&v;let rr=&r;out=*rr;}let oracle_observation0=*out;oracle_observation0}
fn main(){if run()==14{println!("nested references oracle ok");}}
"###,
        expected_stdout: "nested references oracle ok\n",
    },
    Fixture {
        id: "P05",
        name: "generic_reference_free_T_inner_load_and_forward",
        group: "positive",
        source_sha256: "c27ed25e4188a1ed31dc7009508cafdb9a47847fe3491ee97b24b218aa3c7f70",
        full_source: r###"struct Pair<T>{value:T,tag:i32}
fn inner<'o,'i,T>(x:&'o &'i T)->&'i T{*x}
fn forward<'o,'i,T>(x:&'o &'i T)->&'i T{inner(x)}
fn run()->i32{let v=Pair{value:15,tag:1};let r=&v;let out=forward(&r);let oracle_observation0=out.value+out.tag;oracle_observation0}
fn main(){if run()==16{println!("nested references oracle ok");}}
"###,
        expected_stdout: "nested references oracle ok\n",
    },
    Fixture {
        id: "P06",
        name: "plain_shared_carrier_copy_with_noncopy_reference_free_T",
        group: "positive",
        source_sha256: "f1da2cbcee5fc4e9557bd008daa2c0d6f5d2ef31996940cd8345ed2a0246d55a",
        full_source: r###"struct Token{v:i32}
fn run()->i32{let v=Token{v:16};let r=&v;let rr=&r;let copied=rr;let a=*rr;let b=*copied;let oracle_observation0=a.v+b.v;oracle_observation0}
fn main(){if run()==32{println!("nested references oracle ok");}}
"###,
        expected_stdout: "nested references oracle ok\n",
    },
    Fixture {
        id: "P07",
        name: "distinct_named_lifetimes_imply_inner_outlives_outer",
        group: "positive",
        source_sha256: "ea02f0f4d7a1b29983c8d5815e52b002d847eb7ab4d0f3322e08662d7044805b",
        full_source: r###"fn shorten<'o,'i,T>(x:&'o &'i T)->&'o T{*x}
fn run()->i32{let v=17;let r=&v;let out=shorten(&r);let oracle_observation0=*out;oracle_observation0}
fn main(){if run()==17{println!("nested references oracle ok");}}
"###,
        expected_stdout: "nested references oracle ok\n",
    },
    Fixture {
        id: "P08",
        name: "explicit_inner_result_lifetime_not_outer_lifetime",
        group: "positive",
        source_sha256: "569eb5a4807d1ab389586ff8bcdd36a7bb65f2d5fe012be0710dcbbaeeae6ecb",
        full_source: r###"fn inner<'outer,'inner>(x:&'outer &'inner i32)->&'inner i32{*x}
fn run()->i32{let v=18;let out;{let r=&v;out=inner(&r);}let oracle_observation0=*out;oracle_observation0}
fn main(){if run()==18{println!("nested references oracle ok");}}
"###,
        expected_stdout: "nested references oracle ok\n",
    },
    Fixture {
        id: "P09",
        name: "call_returns_inner_with_original_argument_origins",
        group: "positive",
        source_sha256: "9369f2a39ff31a5dc613efafb0b1135b3b89c00fc7425b5dd9b77f859f2a1c10",
        full_source: r###"fn get<'o,'i>(x:&'o &'i i32)->&'i i32{*x}
fn run()->i32{let v=19;let r=&v;let result=get(&r);let oracle_observation0=*result;oracle_observation0}
fn main(){if run()==19{println!("nested references oracle ok");}}
"###,
        expected_stdout: "nested references oracle ok\n",
    },
    Fixture {
        id: "P10",
        name: "genuine_nested_call_once_forwarding_preserves_layer_positions",
        group: "positive",
        source_sha256: "db30a19104db49d6155ab0ce3bc8c9949b081947f321dab754482a2c0beeceaa",
        full_source: r###"fn tick<'o,'i>(x:&'o &'i i32,n:&mut i32)->&'o &'i i32{*n+=1;x}
fn inner<'o,'i>(x:&'o &'i i32)->&'i i32{*x}
fn run()->i32{let v=20;let r=&v;let mut n=0;let out=inner(tick(&r,&mut n));let oracle_observation0=*out;let oracle_observation1=n;oracle_observation0*100+oracle_observation1}
fn main(){if run()==2001{println!("nested references oracle ok");}}
"###,
        expected_stdout: "nested references oracle ok\n",
    },
    Fixture {
        id: "P11",
        name: "joined_inner_load_unions_original_referent_origins",
        group: "positive",
        source_sha256: "842b053c4106c5f85e4843398bd9c714e32406ce5e772c0d2ad4db08cae49f22",
        full_source: r###"fn choose<'o,'i>(a:&'o &'i i32,b:&'o &'i i32,flag:bool)->&'i i32{let rr;if flag{rr=a;}else{rr=b;}*rr}
fn run()->i32{let a=21;let b=22;let ra=&a;let rb=&b;let oracle_observation0=*choose(&ra,&rb,true);let oracle_observation1=*choose(&ra,&rb,false);oracle_observation0*100+oracle_observation1}
fn main(){if run()==2122{println!("nested references oracle ok");}}
"###,
        expected_stdout: "nested references oracle ok\n",
    },
    Fixture {
        id: "P12",
        name: "rebind_nested_carrier_then_load_uses_current_origins",
        group: "positive",
        source_sha256: "bbdbdf38a2b03e8c79419fd0139c511aa71ca929eecbedef9e5194876135ac08",
        full_source: r###"fn run()->i32{let a=23;let b=24;let ra=&a;let rb=&b;let mut rr=&ra;let first=*rr;rr=&rb;let second=*rr;let oracle_observation0=*first;let oracle_observation1=*second;oracle_observation0*100+oracle_observation1}
fn main(){if run()==2324{println!("nested references oracle ok");}}
"###,
        expected_stdout: "nested references oracle ok\n",
    },
    Fixture {
        id: "P13",
        name: "loaded_inner_survives_handle_rebind_to_other_referent",
        group: "positive",
        source_sha256: "d6ce5a0a3c30dc35d2adfe9767434ec0b2f8f59a99d2550792626b6924f7fdcc",
        full_source: r###"fn run()->i32{let a=25;let b=26;let mut handle=&a;let loaded;{let rr=&handle;loaded=*rr;}handle=&b;let oracle_observation0=*loaded;let oracle_observation1=*handle;oracle_observation0*100+oracle_observation1}
fn main(){if run()==2526{println!("nested references oracle ok");}}
"###,
        expected_stdout: "nested references oracle ok\n",
    },
    Fixture {
        id: "P14",
        name: "referent_write_after_loaded_inner_last_use",
        group: "positive",
        source_sha256: "7965081046c18fe6bfbc1546790ac647b5d76d9399ed550abb67552a7d9ddb4d",
        full_source: r###"fn run()->i32{let mut v=27;let r=&v;let rr=&r;let loaded=*rr;let oracle_observation0=*loaded;v=28;let oracle_observation1=v;oracle_observation0*100+oracle_observation1}
fn main(){if run()==2728{println!("nested references oracle ok");}}
"###,
        expected_stdout: "nested references oracle ok\n",
    },
    Fixture {
        id: "P15",
        name: "handle_rebind_after_outer_last_use_while_loaded_inner_lives",
        group: "positive",
        source_sha256: "ee60e56814cdd9bae7cc6db8b3b069b550b7568c4ccdb8951b26e95ae40c9c53",
        full_source: r###"fn run()->i32{let a=29;let b=30;let mut r=&a;let rr=&r;let loaded=*rr;r=&b;let oracle_observation0=*loaded;let oracle_observation1=*r;oracle_observation0*100+oracle_observation1}
fn main(){if run()==2930{println!("nested references oracle ok");}}
"###,
        expected_stdout: "nested references oracle ok\n",
    },
    Fixture {
        id: "P16",
        name: "T_U_concrete_coincidence_preserves_inner_candidate_authority",
        group: "positive",
        source_sha256: "16b62fad6da4ceea4dea00a8e4083ae2dc209593accbbcead117341ef9bde03f",
        full_source: r###"fn choose<'o,'i,'p,'j,T,U>(x:&'o &'i T,y:&'p &'j U)->&'i T{*x}
fn run()->i32{let a=31;let b=32;let ra=&a;let rb=&b;let oracle_observation0=*choose::<i32,i32>(&ra,&rb);oracle_observation0}
fn main(){if run()==31{println!("nested references oracle ok");}}
"###,
        expected_stdout: "nested references oracle ok\n",
    },
    Fixture {
        id: "N17",
        name: "loaded_inner_escapes_original_referent_storage",
        group: "invalid",
        source_sha256: "3d5f1f6ef7e0c17eccb126fc91f6b392c2129969b4434e9e9a33506b39669311",
        full_source: r###"fn run()->i32{let out;{let v=1;let r=&v;let rr=&r;out=*rr;}let oracle_observation0=*out;oracle_observation0}
fn main(){if run()==0{println!("nested references oracle ok");}}
"###,
        expected_stdout: "",
    },
    Fixture {
        id: "N18",
        name: "returned_outer_escapes_local_inner_handle_storage",
        group: "invalid",
        source_sha256: "e1712933cf173ba6bc02397f1309af759de683e91135b2e72b45bf3dfaa661ed",
        full_source: r###"fn bad<'i>(x:&'i i32)->&'i &'i i32{let r=x;&r}
fn run()->i32{0}
fn main(){if run()==0{println!("nested references oracle ok");}}
"###,
        expected_stdout: "",
    },
    Fixture {
        id: "N19",
        name: "live_loaded_inner_blocks_original_referent_write",
        group: "invalid",
        source_sha256: "beb7ef51a0b5024e817633d53ea50fab6894f129b8a4b89f17f22c7d86f3cde6",
        full_source: r###"fn run()->i32{let mut v=1;let r=&v;let rr=&r;let loaded=*rr;v=2;let oracle_observation0=*loaded;oracle_observation0}
fn main(){if run()==0{println!("nested references oracle ok");}}
"###,
        expected_stdout: "",
    },
    Fixture {
        id: "N20",
        name: "live_loaded_inner_blocks_original_referent_move",
        group: "invalid",
        source_sha256: "4ab7c90cb5e293fac00f604941a5d9922d084b77cb34256feb305fe359219b83",
        full_source: r###"struct Token{v:i32}
fn take(x:Token)->i32{x.v}
fn run()->i32{let v=Token{v:1};let r=&v;let rr=&r;let loaded=*rr;let consumed=take(v);let oracle_observation0=loaded.v;let oracle_observation1=consumed;oracle_observation0*100+oracle_observation1}
fn main(){if run()==0{println!("nested references oracle ok");}}
"###,
        expected_stdout: "",
    },
    Fixture {
        id: "N21",
        name: "live_loaded_inner_blocks_original_referent_exclusive_borrow",
        group: "invalid",
        source_sha256: "3eaea23184d59d473889b84b67c14188357329c70557571c0f3c61c906e27cca",
        full_source: r###"fn write(x:&mut i32){*x=2;}
fn run()->i32{let mut v=1;let r=&v;let rr=&r;let loaded=*rr;write(&mut v);let oracle_observation0=*loaded;oracle_observation0}
fn main(){if run()==0{println!("nested references oracle ok");}}
"###,
        expected_stdout: "",
    },
    Fixture {
        id: "N22",
        name: "live_outer_blocks_inner_handle_rebind",
        group: "invalid",
        source_sha256: "cc1abfee3ce816246a46ea906d37dca5eab9bad40b2af68bcce01f6803a3951c",
        full_source: r###"fn run()->i32{let a=1;let b=2;let mut r=&a;let rr=&r;r=&b;let oracle_observation0=**rr;oracle_observation0}
fn main(){if run()==0{println!("nested references oracle ok");}}
"###,
        expected_stdout: "",
    },
    Fixture {
        id: "N23",
        name: "uninitialized_outer_carrier_inner_load",
        group: "invalid",
        source_sha256: "f7dfde5dc385cfdc0f3cadc55f5e93c1f5a0e447416b3b50aea425b813b2a5c4",
        full_source: r###"fn run()->i32{let rr:&&i32;let loaded=*rr;let oracle_observation0=*loaded;oracle_observation0}
fn main(){if run()==0{println!("nested references oracle ok");}}
"###,
        expected_stdout: "",
    },
    Fixture {
        id: "N24",
        name: "joined_inner_retains_expired_branch_origin",
        group: "invalid",
        source_sha256: "8f5b26e384c1830cd7b20ca44fef72ed95ea95656cd06e62287391d732ca60aa",
        full_source: r###"fn run()->i32{let a=1;let ra=&a;let out;{let b=2;let rb=&b;let rr;if a==1{rr=&ra;}else{rr=&rb;}out=*rr;}let oracle_observation0=*out;oracle_observation0}
fn main(){if run()==0{println!("nested references oracle ok");}}
"###,
        expected_stdout: "",
    },
    Fixture {
        id: "N25",
        name: "loaded_inner_rebind_does_not_release_old_live_origin",
        group: "invalid",
        source_sha256: "cabfa0ffe3c996cc56791fa515dbcf8af9c68f02507a300657c94a2398075a91",
        full_source: r###"fn run()->i32{let mut a=1;let b=2;let ra=&a;let rb=&b;let mut rr=&ra;let old=*rr;rr=&rb;a=3;let oracle_observation0=*old;let oracle_observation1=**rr;oracle_observation0*100+oracle_observation1}
fn main(){if run()==0{println!("nested references oracle ok");}}
"###,
        expected_stdout: "",
    },
    Fixture {
        id: "N26",
        name: "unused_generic_body_returns_inner_under_unrelated_result_region",
        group: "invalid",
        source_sha256: "061a9a6179fd36020249952ee5a945e852588f69cebbf41ea3c9848bf20497a4",
        full_source: r###"fn bad<'o,'i,'unrelated,T>(x:&'o &'i T)->&'unrelated T{*x}
fn run()->i32{0}
fn main(){if run()==0{println!("nested references oracle ok");}}
"###,
        expected_stdout: "",
    },
    Fixture {
        id: "N27",
        name: "implied_inner_outlives_outer_does_not_prove_reverse",
        group: "invalid",
        source_sha256: "58ede58052e422df7988a18eb6213d749659121b11a2c44d0b461e6f5a4f014b",
        full_source: r###"fn bad<'o,'i,T>(x:&'o &'i T)->&'i &'i T{x}
fn run()->i32{0}
fn main(){if run()==0{println!("nested references oracle ok");}}
"###,
        expected_stdout: "",
    },
    Fixture {
        id: "N28",
        name: "wrong_inner_position_candidate_despite_T_U_coincidence",
        group: "invalid",
        source_sha256: "61aaf0f369d662e156368cf37217ac25da12defcb2cfa28cee00cf26dbcc82fb",
        full_source: r###"fn bad<'o,'i,'p,'j,T,U>(x:&'o &'i T,y:&'p &'j U)->&'i T{*y}
fn run()->i32{let a=1;let b=2;let ra=&a;let rb=&b;let _=bad::<i32,i32>(&ra,&rb);0}
fn main(){if run()==0{println!("nested references oracle ok");}}
"###,
        expected_stdout: "",
    },
    Fixture {
        id: "X29",
        name: "nested_mutable_reference_chain",
        group: "legal-excluded",
        source_sha256: "0cd40686bbbe209e74df6e40de6bac1084f49ecf6f7cca7efa352b5875cba99d",
        full_source: r###"fn run()->i32{let mut v=41;let r=&mut v;let rr=&r;let oracle_observation0=**rr;oracle_observation0}
fn main(){if run()==41{println!("nested references oracle ok");}}
"###,
        expected_stdout: "nested references oracle ok\n",
    },
    Fixture {
        id: "X30",
        name: "aggregate_reference_fields",
        group: "legal-excluded",
        source_sha256: "502c3ee736ac7be89edde1b63cc0306067e1797467fa27529848433d91529a9f",
        full_source: r###"struct Holder<'a>{r:&'a i32}
fn run()->i32{let v=42;let h=Holder{r:&v};let oracle_observation0=*h.r;oracle_observation0}
fn main(){if run()==42{println!("nested references oracle ok");}}
"###,
        expected_stdout: "nested references oracle ok\n",
    },
    Fixture {
        id: "X31",
        name: "ordinary_reference_generic_actual",
        group: "legal-excluded",
        source_sha256: "80ae5b278b3d3fb1165e17e54bc23ffbc9238019e0b347464035f08a6fc95070",
        full_source: r###"fn id<T>(x:T)->T{x}
fn run()->i32{let v=43;let r=id::<&i32>(&v);let oracle_observation0=*r;oracle_observation0}
fn main(){if run()==43{println!("nested references oracle ok");}}
"###,
        expected_stdout: "nested references oracle ok\n",
    },
    Fixture {
        id: "X32",
        name: "shared_reference_depth_greater_than_two",
        group: "legal-excluded",
        source_sha256: "677de66b1201fd9d44cc3a8b133897b4076c0a5c36ded40a57a877877c1d82b0",
        full_source: r###"fn run()->i32{let v=44;let r=&v;let rr=&r;let rrr=&rr;let oracle_observation0=***rrr;oracle_observation0}
fn main(){if run()==44{println!("nested references oracle ok");}}
"###,
        expected_stdout: "nested references oracle ok\n",
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
fn frozen_nested_shared_positive16() {
    for fixture in group("positive", 16) {
        runs(fixture.full_source, fixture.expected_stdout);
    }
}

#[test]
fn frozen_nested_shared_invalid12() {
    for fixture in group("invalid", 12) {
        rejects(fixture.name, fixture.full_source, true);
    }
}

#[test]
fn frozen_nested_shared_legal_excluded4() {
    for fixture in group("legal-excluded", 4) {
        rejects(fixture.name, fixture.full_source, false);
    }
}
