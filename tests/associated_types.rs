//! Frozen stock-first closed projection witnesses for ASSOCIATED-TYPES.md.
//! Native categories are exact-source stock/native measurements, rechecked 2026-10-19.
//! First18 JSON SHA256 ab503e0282894e9a3c55229f1179225198499225992a4df1b2b247fcebca2293.
//! Append9 JSON SHA256 bf933985f8a1a93f75a7ecce9489727eeb6dc2d28eb88ab39bdccf734fa90e43.
//! Literal append12 JSON SHA256 ac05abbe21575c3a1d16d66a9aadd849ad45ec176de4f8015beeee4991bb16a7.
//! Sources retain original 01..27 bytes/order and append 28..39, including final LF.
//! Original cases 01,03,04,15..17,19,21..23,26,27 now match stock output.
//! Case16 witnesses this closed reference projection, not general reference parity.
//! Case07 stops at unresolved String, before checking its associated Copy obligation.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
use justrust_engine::compiler::{frontend, source};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
const BASELINE_STOCK: &str = "rustc 1.98.1;48a229ceaefd4985c50990b14116b6d856af0985;2026-09-01;x86_64-unknown-linux-gnu;LLVM22.1.8";
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    Runtime,
    Invalid,
    Unsupported,
}
struct Case {
    name: &'static str,
    source: &'static str,
    stdout: Option<&'static str>,
    stock_diagnostic: Option<&'static str>,
    outcome: Outcome,
}
fn cases() -> Vec<Case> {
    vec![
        // Source SHA256 86f0e9cce1d8ec1ab66077a9fe4b943544ab85ccba6223987540dd1f344df664
        Case {
            name: "01_concrete_signature",
            source: r#"trait Has<T>{type Out;} struct S; impl Has<i32> for S{type Out=i32;} fn take(x:<S as Has<i32>>::Out)->i32{x+1} fn main(){println!("{}",take(41));}
"#,
            stdout: Some("42\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        // Source SHA256 15f4e9033f3e0f0d95e0e8cdb9dd08fad6f24c412954d078171f7aedafc7437e
        Case {
            name: "02_same_self_distinct_argument",
            source: r#"trait Has<T>{type Out;} struct S; impl Has<i32> for S{type Out=i32;} impl Has<bool> for S{type Out=bool;} fn main(){let a:<S as Has<i32>>::Out=42; let b:<S as Has<bool>>::Out=true; println!("{a} {b}");}
"#,
            stdout: Some("42 true\n"),
            stock_diagnostic: None,
            outcome: Outcome::Unsupported,
        },
        // Source SHA256 a34c29b39d1eec99304af9df7a8e9634cb8d76e4ce35911289ed79a563ea85f6
        Case {
            name: "03_generic_impl_closed",
            source: r#"trait Has{type Out;} struct Wrap<T>(T); impl<T> Has for Wrap<T>{type Out=T;} fn take(x:<Wrap<i32> as Has>::Out)->i32{x} fn main(){println!("{}",take(42));}
"#,
            stdout: Some("42\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        // Source SHA256 e86817c10961ae9b4dbbbb826160d13820493c77eec9b331e7b80c2fabe6f0e6
        Case {
            name: "04_explicit_ufcs_self_out",
            source: r#"trait Has{type Out; fn get()->Self::Out;} struct S; impl Has for S{type Out=i32; fn get()->Self::Out{42}} fn main(){println!("{}",<S as Has>::get());}
"#,
            stdout: Some("42\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        // Source SHA256 f50bdc6a656d5aad155c7b4d492bfed30d43b6618e16cc82ed31d0c335fb6b55
        Case {
            name: "05_missing_assignment",
            source: r#"trait Has{type Out;} struct S; impl Has for S{} fn main(){}
"#,
            stdout: None,
            stock_diagnostic: Some("error[E0046]"),
            outcome: Outcome::Invalid,
        },
        // Source SHA256 c1f7a4c4c65ad21b747c0e5683e8d5c5b934ca01ad468add137a1015f659a653
        Case {
            name: "06_duplicate_assignment",
            source: r#"trait Has{type Out;} struct S; impl Has for S{type Out=i32; type Out=bool;} fn main(){}
"#,
            stdout: None,
            stock_diagnostic: Some("error[E0201]"),
            outcome: Outcome::Invalid,
        },
        // Source SHA256 1f00728d9d8af0f079d5276f79035dd47075300ee114f5bfff9fed63ebd5c782
        Case {
            name: "07_wrong_assignment",
            source: r#"trait Has{type Out: Copy;} struct S; impl Has for S{type Out=String;} fn main(){}
"#,
            stdout: None,
            stock_diagnostic: Some("error[E0277]"),
            outcome: Outcome::Invalid,
        },
        // Source SHA256 d1b6874d1ca32f13a83c0b68a7ea2b3e7662cfb02480c3756bc9df0d362b499c
        Case {
            name: "08_wrong_method_signature",
            source: r#"trait Has{type Out; fn get()->Self::Out;} struct S; impl Has for S{type Out=i32; fn get()->bool{true}} fn main(){}
"#,
            stdout: None,
            stock_diagnostic: Some("error[E0053]"),
            outcome: Outcome::Invalid,
        },
        // Source SHA256 0c7cbb11fbe5dec4f995b383d66bbf4af14a727ce732e9c3078c3ecf4d3a535f
        Case {
            name: "09_unused_bad_rhs",
            source: r#"trait Has{type Out;} struct S; impl Has for S{type Out=Missing;} fn main(){}
"#,
            stdout: None,
            stock_diagnostic: Some("error[E0425]"),
            outcome: Outcome::Invalid,
        },
        // Source SHA256 67a1a237e415f45ed87e8c76df8e2b46bb9d6a0e3632d167941741f4e4dc094e
        Case {
            name: "10_direct_cycle",
            source: r#"trait Has{type Out;} struct S; impl Has for S{type Out=<S as Has>::Out;} fn main(){}
"#,
            stdout: None,
            stock_diagnostic: Some("error[E0275]"),
            outcome: Outcome::Invalid,
        },
        // Source SHA256 1b64ab6258a57770c1f914468e3e5b4b529a8df9c3c4f9d3dea040b727414d6f
        Case {
            name: "11_mutual_cycle",
            source: r#"trait Has{type Out;} struct S; struct U; impl Has for S{type Out=<U as Has>::Out;} impl Has for U{type Out=<S as Has>::Out;} fn main(){}
"#,
            stdout: None,
            stock_diagnostic: Some("error[E0275]"),
            outcome: Outcome::Invalid,
        },
        // Source SHA256 48166483fd308fcc20a7a3bc7ea24070c6f718945fe4ab3f7d9fa3f8464ff5f2
        Case {
            name: "12_foreign_missing_item",
            source: r#"struct S; impl Iterator for S{fn next(&mut self)->Option<Self::Item>{None}} fn main(){}
"#,
            stdout: None,
            stock_diagnostic: Some("error[E0046]"),
            outcome: Outcome::Invalid,
        },
        // Source SHA256 69fcc440b9df353ac626e7dc4dc760ee61b0277f3c5b9c0196b44876e9f51160
        Case {
            name: "13_trait_arity",
            source: r#"trait Has<T>{type Out;} struct S; impl Has<i32> for S{type Out=i32;} fn take(_: <S as Has>::Out){} fn main(){}
"#,
            stdout: None,
            stock_diagnostic: Some("error[E0107]"),
            outcome: Outcome::Invalid,
        },
        // Source SHA256 c9a4658a5b62c33abddbdbcc8ef756443a7af86bfbcdf1817a36202f4de50c0b
        Case {
            name: "14_generic_rigid_forward",
            source: r#"trait Has{type Out;} fn forward<T:Has>(x:T::Out)->T::Out{x} struct S; impl Has for S{type Out=i32;} fn main(){println!("{}",forward::<S>(42));}
"#,
            stdout: Some("42\n"),
            stock_diagnostic: None,
            outcome: Outcome::Unsupported,
        },
        // Source SHA256 73a148f8dee43259d2e33898685d7570ef47b360a6d094abd68948a3877fc6ab
        Case {
            name: "15_closed_nested_projection",
            source: r#"trait Has{type Out;} struct S; struct U; impl Has for S{type Out=U;} impl Has for U{type Out=i32;} fn take(x:<<S as Has>::Out as Has>::Out)->i32{x} fn main(){println!("{}",take(42));}
"#,
            stdout: Some("42\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        // Source SHA256 92c6be4b1df4de6e17180a36a72861ac0cd7e88c5183ede2700c33d6f7284eb8
        Case {
            name: "16_closed_reference_projection",
            source: r#"trait Has{type Out;} struct S; impl Has for S{type Out=i32;} fn take(x:&<S as Has>::Out)->i32{*x} fn main(){println!("{}",take(&42));}
"#,
            stdout: Some("42\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        // Source SHA256 a369e02b0ee590d856c7e27d0ea4c70cc9bdadb4bc9dfb852514893604339fd9
        Case {
            name: "17_closed_tuple_projection",
            source: r#"trait Has{type Out;} struct S; impl Has for S{type Out=(i32,bool);} fn main(){let x:<S as Has>::Out=(42,true); println!("{} {}",x.0,x.1);}
"#,
            stdout: Some("42 true\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        // Source SHA256 aef6e467bc36c90fcdd49673a740a029f67553788e8312cf2e7398b4aaa38062
        Case {
            name: "18_wrong_closed_value",
            source: r#"trait Has{type Out;} struct S; impl Has for S{type Out=i32;} fn take(_: <S as Has>::Out){} fn main(){take(true);}
"#,
            stdout: None,
            stock_diagnostic: Some("error[E0308]"),
            outcome: Outcome::Invalid,
        },
        // Source SHA256 84445e59054dabb32eb18239c73797f008d66ad829c59e75dd96dc1152af97f3
        Case {
            name: "19_general_tuple",
            source: r#"trait Has{type Out;} struct S; impl Has for S{type Out=(i32,bool,i64);} fn pass(x:<S as Has>::Out)-><S as Has>::Out{x} fn main(){let x=pass((42,true,9000)); println!("{} {} {}",x.0,x.1,x.2);}
"#,
            stdout: Some("42 true 9000\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        // Source SHA256 be357731ced256bd685158d2e236bb3e437037002c1198116a3cbee79c799bcc
        Case {
            name: "20_copy_record",
            source: r#"trait Has{type Out;} #[derive(Clone,Copy)] struct Record{n:i32,b:bool} struct S; impl Has for S{type Out=Record;} fn pass(x:<S as Has>::Out)-><S as Has>::Out{x} fn main(){let x=Record{n:42,b:true}; let y=pass(x); println!("{} {} {}",x.n,y.n,y.b);}
"#,
            stdout: Some("42 42 true\n"),
            stock_diagnostic: None,
            outcome: Outcome::Unsupported,
        },
        // Source SHA256 b3375369a912b50b7709f0b867125b59413e9d6a19b40f0d23dd1fdae07a18ab
        Case {
            name: "21_noncopy_cell",
            source: r#"trait Has{type Out;} struct Cell{n:i32} struct S; impl Has for S{type Out=Cell;} fn pass(x:<S as Has>::Out)-><S as Has>::Out{x} fn main(){let y=pass(Cell{n:42}); println!("{}",y.n);}
"#,
            stdout: Some("42\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        // Source SHA256 a95d5a936e39e090c83aa672f0b5997b85a6b0c18f4611585e03b319d0642d75
        Case {
            name: "22_wide_i128",
            source: r#"trait Has{type Out;} struct S; impl Has for S{type Out=i128;} fn pass(x:<S as Has>::Out)-><S as Has>::Out{x} fn main(){println!("{}",pass(1267650600228229401496703205376i128));}
"#,
            stdout: Some("1267650600228229401496703205376\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        // Source SHA256 0ab7b94e8276c1d008428d154fb4ee35dd2a95f0a08a72adfc6044d8bd04d900
        Case {
            name: "23_generic_tuple_closed",
            source: r#"trait Has{type Out;} struct Wrap<T>(T); impl<T> Has for Wrap<T>{type Out=(T,bool);} fn pass(x:<Wrap<i32> as Has>::Out)-><Wrap<i32> as Has>::Out{x} fn main(){let y=pass((42,true)); println!("{} {}",y.0,y.1);}
"#,
            stdout: Some("42 true\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        // Source SHA256 f50bdc6a656d5aad155c7b4d492bfed30d43b6618e16cc82ed31d0c335fb6b55
        Case {
            name: "24_missing_assignment_unbounded",
            source: r#"trait Has{type Out;} struct S; impl Has for S{} fn main(){}
"#,
            stdout: None,
            stock_diagnostic: Some("error[E0046]"),
            outcome: Outcome::Invalid,
        },
        // Source SHA256 5c11bc016f401a430c3ff13b08bea4e258b52bf615e7ae65f04d357aac5b2e1b
        Case {
            name: "25_extra_assignment_unbounded",
            source: r#"trait Has{type Out;} struct S; impl Has for S{type Out=i32; type Extra=bool;} fn main(){}
"#,
            stdout: None,
            stock_diagnostic: Some("error[E0437]"),
            outcome: Outcome::Invalid,
        },
        // Source SHA256 ab18014a0ca0f4c4b5775e560f161f07b979c32cc44651fc0d4a4751413f43da
        Case {
            name: "26_nested_closed_chain",
            source: r#"trait Has{type Out;} struct A; struct B; struct C; impl Has for A{type Out=B;} impl Has for B{type Out=C;} impl Has for C{type Out=i32;} fn pass(x:<<<A as Has>::Out as Has>::Out as Has>::Out)->i32{x} fn main(){println!("{}",pass(42));}
"#,
            stdout: Some("42\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        // Source SHA256 e5ff7c3e0dfe5391ad8b76a86eb479a22079654eb240e33e71c13cc75b02af24
        Case {
            name: "27_qualified_projection_alias",
            source: r#"trait Has<T>{type Out;} struct S; impl Has<i32> for S{type Out=(i32,bool);} type Closed=<S as Has<i32>>::Out; fn pass(x:Closed)->Closed{x} fn main(){let y=pass((42,true)); println!("{} {}",y.0,y.1);}
"#,
            stdout: Some("42 true\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        // Source SHA256 0be435af31b06a64d9cb8e969d6d60c26f8dcbcdaaf290807abdb9d7ae717c76
        Case {
            name: "28_literal_concrete_signature",
            source: r#"trait Has<T>{type Out;} struct S; impl Has<i32> for S{type Out=i32;} fn take(x:<S as Has<i32>>::Out)->i32{x+1} fn main(){if take(41)==42{println!("ok");}}
"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        // Source SHA256 0b0349825eb5382c232b04b00a235691c4d31a14ca0e7027d268242dedaee9e0
        Case {
            name: "29_literal_same_self_distinct_argument",
            source: r#"trait Has<T>{type Out;} struct S; impl Has<i32> for S{type Out=i32;} impl Has<bool> for S{type Out=bool;} fn main(){let a:<S as Has<i32>>::Out=42; let b:<S as Has<bool>>::Out=true; if a==42{if b{println!("ok");}}}
"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        // Source SHA256 7e1d7a1c23f5382a3b21766e9d3951faeea43358b3ab9091f86025a63db3c5ed
        Case {
            name: "30_literal_generic_impl_closed",
            source: r#"trait Has{type Out;} struct Wrap<T>(T); impl<T> Has for Wrap<T>{type Out=T;} fn take(x:<Wrap<i32> as Has>::Out)->i32{x} fn main(){if take(42)==42{println!("ok");}}
"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        // Source SHA256 94def1b8f67cf617a5522643ca80a25e56f973b9e2b1549e3b555444fea2f3c9
        Case {
            name: "31_literal_closed_nested_projection",
            source: r#"trait Has{type Out;} struct S; struct U; impl Has for S{type Out=U;} impl Has for U{type Out=i32;} fn take(x:<<S as Has>::Out as Has>::Out)->i32{x} fn main(){if take(42)==42{println!("ok");}}
"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        // Source SHA256 eadbc889029a71f657fb466b757ab5c5fefa5dd539c4070802fa5dedcb8b1e95
        Case {
            name: "32_literal_closed_tuple_projection",
            source: r#"trait Has{type Out;} struct S; impl Has for S{type Out=(i32,bool);} fn main(){let x:<S as Has>::Out=(42,true); if x.0==42{if x.1{println!("ok");}}}
"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        // Source SHA256 fea49ad22dd6827a4dd458db6a040da316944e8d1584db45371fe60c0dbde33d
        Case {
            name: "33_literal_general_tuple",
            source: r#"trait Has{type Out;} struct S; impl Has for S{type Out=(i32,bool,i64);} fn pass(x:<S as Has>::Out)-><S as Has>::Out{x} fn main(){let x=pass((42,true,9000)); if x.0==42{if x.1{if x.2==9000{println!("ok");}}}}
"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        // Source SHA256 73ebe2162c880b0fc8ec6114feeec82e4faf94165b53bebdfcd17ab260f244e5
        Case {
            name: "34_literal_copy_record",
            source: r#"trait Has{type Out;} #[derive(Clone,Copy)] struct Record{n:i32,b:bool} struct S; impl Has for S{type Out=Record;} fn pass(x:<S as Has>::Out)-><S as Has>::Out{x} fn main(){let x=Record{n:42,b:true}; let y=pass(x); if x.n==42{if x.b{if y.n==42{if y.b{println!("ok");}}}}}
"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Unsupported,
        },
        // Source SHA256 f659b344f6565bf0436ad3c079a30027df3e565cc13d706c0db2ec95c5fdf784
        Case {
            name: "35_literal_noncopy_cell",
            source: r#"trait Has{type Out;} struct Cell{n:i32} struct S; impl Has for S{type Out=Cell;} fn pass(x:<S as Has>::Out)-><S as Has>::Out{x} fn main(){let y=pass(Cell{n:42}); if y.n==42{println!("ok");}}
"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        // Source SHA256 914e71656848e36224d96ed35db22630d521a867d987364b2c587a12d72e7ab4
        Case {
            name: "36_literal_wide_i128",
            source: r#"trait Has{type Out;} struct S; impl Has for S{type Out=i128;} fn pass(x:<S as Has>::Out)-><S as Has>::Out{x} fn main(){let y=pass(1267650600228229401496703205376i128); if y==1267650600228229401496703205376i128{println!("ok");}}
"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        // Source SHA256 bad9add94dc2cda3151f836b0052050d7e372cb57e07b12d44e8f0ba51656730
        Case {
            name: "37_literal_generic_tuple_closed",
            source: r#"trait Has{type Out;} struct Wrap<T>(T); impl<T> Has for Wrap<T>{type Out=(T,bool);} fn pass(x:<Wrap<i32> as Has>::Out)-><Wrap<i32> as Has>::Out{x} fn main(){let y=pass((42,true)); if y.0==42{if y.1{println!("ok");}}}
"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        // Source SHA256 0dc6a273e32a5dbec808faeae8a80b079f6907b1a1f2758e4aa011151a783b74
        Case {
            name: "38_literal_nested_closed_chain",
            source: r#"trait Has{type Out;} struct A; struct B; struct C; impl Has for A{type Out=B;} impl Has for B{type Out=C;} impl Has for C{type Out=i32;} fn pass(x:<<<A as Has>::Out as Has>::Out as Has>::Out)->i32{x} fn main(){if pass(42)==42{println!("ok");}}
"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        // Source SHA256 5b82ff1227784043a0ced5e34910dc7e77b41f0a8b2bcf0d33c1f6a91bd2a69d
        Case {
            name: "39_literal_qualified_projection_alias",
            source: r#"trait Has<T>{type Out;} struct S; impl Has<i32> for S{type Out=(i32,bool);} type Closed=<S as Has<i32>>::Out; fn pass(x:Closed)->Closed{x} fn main(){let y=pass((42,true)); if y.0==42{if y.1{println!("ok");}}}
"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        // Source SHA256 c82eace862626acca532cca162e1eab9308763dadec84ca39cd6b3f4fc88ab26
        Case {
            name: "40_manual_copy_record",
            source: r#"trait Has{type Out;} struct Record{n:i32,b:bool} impl Clone for Record{fn clone(&self)->Self{Record{n:self.n,b:self.b}}} impl Copy for Record{} struct S; impl Has for S{type Out=Record;} fn pass(x:<S as Has>::Out)-><S as Has>::Out{x} fn main(){let x=Record{n:42,b:true}; let y=pass(x); if x.n==42{if x.b{if y.n==42{if y.b{println!("ok");}}}}}
"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
    ]
}
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let base = std::env::var_os("JCODE_SCRATCH_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = base.join(format!(
            "jr-associated-types-{}-{nonce}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
    fn write(&self, name: &str, bytes: &[u8]) {
        let mut file = new_file(&self.path(name));
        file.write_all(bytes).unwrap();
        drop(file);
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
    code: i32,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}
fn capture(command: &mut Command, scratch: &Scratch, seconds: u64) -> Captured {
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let out = scratch.path(&format!("stdout-{id}"));
    let err = scratch.path(&format!("stderr-{id}"));
    command
        .stdin(Stdio::null())
        .stdout(new_file(&out))
        .stderr(new_file(&err));
    let mut retries = 0;
    let child = loop {
        match command.spawn() {
            Ok(child) => break child,
            // Retry only Linux pre-spawn ETXTBSY, never an executed child.
            Err(error) if error.raw_os_error() == Some(26) && retries < 20 => {
                retries += 1;
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(error) => panic!("spawn {command:?}: {error}"),
        }
    };
    let mut child = Running(child);
    let start = Instant::now();
    let status = loop {
        for path in [&out, &err] {
            assert!(
                fs::metadata(path).unwrap().len() <= 65536,
                "output bound: {command:?}"
            );
        }
        if let Some(status) = child.0.try_wait().unwrap() {
            break status;
        }
        assert!(
            start.elapsed() < Duration::from_secs(seconds),
            "timeout: {command:?}"
        );
        std::thread::sleep(Duration::from_millis(5));
    };
    for path in [&out, &err] {
        assert!(fs::metadata(path).unwrap().len() <= 65536);
    }
    Captured {
        code: status.code().expect("signal termination"),
        stdout: fs::read(out).unwrap(),
        stderr: fs::read(err).unwrap(),
    }
}
fn runtime(path: &Path, expected: &str, scratch: &Scratch) {
    assert!(!expected.is_empty(), "runtime oracle cannot be empty");
    let image = fs::read(path).unwrap();
    assert!(image.len() >= 64);
    assert_eq!(&image[..6], b"\x7fELF\x02\x01");
    let result = capture(&mut Command::new(path), scratch, 5);
    assert_eq!(result.code, 0, "runtime: {result:?}");
    assert_eq!(result.stdout, expected.as_bytes());
    assert!(result.stderr.is_empty(), "{result:?}");
}
fn stock_all() {
    let scratch = Scratch::new();
    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let version = capture(Command::new(&rustc).arg("-vV"), &scratch, 10);
    assert_eq!(version.code, 0);
    eprintln!(
        "ASSOCIATED TYPES BASELINE:{BASELINE_STOCK}\nACTUAL STOCK:\n{}",
        String::from_utf8_lossy(&version.stdout)
    );
    let mut counts = (0, 0);
    let mut partition = (0, 0, 0, 0);
    for case in cases() {
        match case.outcome {
            Outcome::Runtime => partition.0 += 1,
            Outcome::Invalid => partition.1 += 1,
            Outcome::Unsupported => partition.2 += 1,
        }
        let scratch = Scratch::new();
        scratch.write("client.rs", case.source.as_bytes());
        let image = scratch.path("stock");
        let result = capture(
            Command::new(&rustc)
                .env_remove("RUSTC_BOOTSTRAP")
                .args([
                    "--edition=2024",
                    "--crate-name=associated_types_oracle",
                    "--color=never",
                    "-Cpanic=abort",
                ])
                .arg(scratch.path("client.rs"))
                .arg("-o")
                .arg(&image),
            &scratch,
            20,
        );
        assert!(result.stdout.is_empty(), "{}:{result:?}", case.name);
        if let Some(expected) = case.stdout {
            counts.0 += 1;
            assert!(case.stock_diagnostic.is_none());
            assert_eq!(result.code, 0, "{}:{result:?}", case.name);
            runtime(&image, expected, &scratch);
        } else {
            counts.1 += 1;
            assert_eq!(result.code, 1, "{}:{result:?}", case.name);
            let diagnostic = String::from_utf8_lossy(&result.stderr);
            assert!(
                diagnostic.contains(case.stock_diagnostic.expect("measured invalid category")),
                "{}:{diagnostic}",
                case.name
            );
            assert!(!image.exists());
        }
    }
    assert_eq!(counts, (28, 12));
    assert_eq!(partition, (24, 12, 4, 0));
}
#[test]
fn stock_nested_associated_types_oracle_edition2024_actual_toolchain() {
    stock_all();
}
#[test]
fn original_stock_valid_sources_keep_their_measured_phases() {
    for case in cases()
        .into_iter()
        .take(27)
        .filter(|case| case.stdout.is_some())
    {
        let parsed = source::prepare_text(
            "projection.rs",
            case.source,
            &source::SourceOptions::default(),
        )
        .map_err(|error| error.to_string())
        .and_then(|prepared| {
            frontend::parse_prepared(&prepared).map_err(|error| error.to_string())
        });
        if case.outcome == Outcome::Runtime {
            parsed.expect(case.name);
        } else {
            let error = parsed.expect_err(case.name);
            assert!(
                error.contains(unsupported_boundary(&case)),
                "{}:{error}",
                case.name
            );
        }
    }
}
fn unsupported_boundary(case: &Case) -> &'static str {
    match case.name {
        "02_same_self_distinct_argument" => "only bare {} formatting placeholders are supported",
        "14_generic_rigid_forward" => "associated type projections are unsupported",
        "20_copy_record" | "34_literal_copy_record" => "active attribute `derive` is unsupported",
        _ => panic!("unclassified Unsupported fixture: {}", case.name),
    }
}
#[test]
fn native_associated_types_workers1_8_check_build_no_compiler_fallback() {
    // jr-rust --profile emits telemetry, not optimization. The coordinator runs
    // this same target under debug and release BOOTSTRAP compiler profiles.
    // Neither run asserts a nonexistent optimized native code-generation mode.
    stock_all();
    let mut gates = 0;
    let mut modes = (0, 0, 0, 0, 0, 0, 0, 0);
    for case in cases() {
        for workers in [1, 8] {
            for check in [true, false] {
                let scratch = Scratch::new();
                scratch.write("client.rs", case.source.as_bytes());
                let image = scratch.path("native");
                let mut command = Command::new(env!("CARGO_BIN_EXE_jr-rust"));
                command
                    .current_dir(&scratch.0)
                    .env_remove("RUSTC_BOOTSTRAP")
                    .env("RUSTC", scratch.path("no-rustc"))
                    .env("PATH", scratch.path("no-tools"))
                    .arg(scratch.path("client.rs"))
                    .args(["--workers", &workers.to_string()]);
                if check {
                    command.arg("--check");
                } else {
                    command.arg("-o").arg(&image);
                }
                let result = capture(&mut command, &scratch, 20);
                let diagnostic = String::from_utf8_lossy(&result.stderr);
                assert!(
                    !diagnostic.contains("panicked at"),
                    "{}:{diagnostic}",
                    case.name
                );
                assert_eq!(
                    result.code,
                    i32::from(case.outcome != Outcome::Runtime),
                    "{} workers{workers} check{check}:{result:?}",
                    case.name
                );
                if case.outcome != Outcome::Runtime {
                    assert!(result.stdout.is_empty());
                    assert!(!diagnostic.is_empty());
                    if case.outcome == Outcome::Unsupported {
                        assert!(
                            diagnostic.contains(unsupported_boundary(&case)),
                            "{}:{diagnostic}",
                            case.name
                        );
                    } else {
                        assert!(
                            !diagnostic.contains("unsupported:"),
                            "{} ordinary invalid must not become Unsupported:{diagnostic}",
                            case.name
                        );
                        if case.name == "07_wrong_assignment" {
                            assert!(
                                diagnostic.contains(": unresolved type\n"),
                                "{} expected name-resolution failure before Copy validation:{diagnostic}",
                                case.name
                            );
                        }
                    }
                }
                if !check && case.outcome == Outcome::Runtime {
                    runtime(
                        &image,
                        case.stdout.expect("runtime source oracle"),
                        &scratch,
                    );
                } else {
                    for path in ["native", "client", "a.out"] {
                        assert!(
                            !scratch.path(path).exists(),
                            "artifact {path} {}",
                            case.name
                        );
                    }
                }
                match (case.outcome, check) {
                    (Outcome::Runtime, true) => modes.0 += 1,
                    (Outcome::Runtime, false) => modes.1 += 1,
                    (Outcome::Invalid, true) => modes.2 += 1,
                    (Outcome::Invalid, false) => modes.3 += 1,
                    (Outcome::Unsupported, true) => modes.4 += 1,
                    (Outcome::Unsupported, false) => modes.5 += 1,
                }
                gates += 1;
            }
        }
    }
    assert_eq!(gates, 160);
    assert_eq!(modes, (48, 48, 24, 24, 8, 8, 0, 0));
    eprintln!(
        "NATIVE ASSOCIATED TYPES:160 cells, modes{modes:?};48 nonempty-output main images;16 stock-valid Unsupported cells remain distinct boundaries;case07 is unresolved String, not Copy-obligation parity or full Rust support"
    );
}
