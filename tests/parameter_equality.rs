//! Frozen parameter-equality stock corpus for PARAMETER-EQUALITY.md / PROJECTED-METHODS.md.
//! Receipt root: /home/jeremy/.jcode/scratch/
//! parameter-rhs-bc428d8aa-oracle/oracle.json (14):
//! f4da57c52ed0930976109226f90a8043cf1583168489ed50daada7d2830659a2
//! parameter-rhs-bc428d8aa-oracle/tuple-rhs-supplement.json (1):
//! b2f758200dc0a176edc109a7d2e9a67ce8f0c208469b1ae68f4dca63191204b7
//! parameter-rhs-bc428d8aa-positive-only-append/positive-only-append.json (9):
//! 0435110eaa65ac16993ec9175e7ca30aa550919de78e6eb93c6c38207c824acf
//! parameter-rhs-negative-only-append/receipt.json (6):
//! efbbd89a6f8fd9d218d6d7f5b119de7d0508f8f1f912deffd61340a85c0b5572
//! Exact original14, supplement1, positive9, negative6 order and final LF retained.
//! Ordered source SHA256 (concatenated UTF-8 sources, no separators):
//! 4161ec41b00e1503931a1b35bd1761b46fdd115aa55e4f956739ff3124bc2ea0
//! Batch concatenated-source SHA256, in the same order:
//! a5ac5e12a1b2b282cc0ca6a7b042dadc4d2c0b51531945cd67aa15c8f38d8aad
//! 998294cd494de913542e86122d54a795710d6592890f31514f7e88642a74acd5
//! 7b7b5b24bea089461c97b9459998d0634c5350ce7f4e7e360b067e425368f7dd
//! e523fba5579aec6d5cc1bddf259232b722070f83d53531ee6917c4c071f12cc1
//! Original receipts use edition2021, default panic; appended receipts edition2024, panic=abort.
//! Additional15 remove ONLY the outer allow attribute from positive9 + negative6.
//! Receipt: parameter-equality-attribute-stripped-20261017/receipt.json
//! SHA256 ac25163924d481b400984bb278a858f834fbb1249fba2850a97bc150eb652407
//! Additional15 concatenated-source SHA256:
//! 5c61d6faabafe8b1b748f22f28cc9a0b3cd43d6ce2222dcde5a08ecc60947ec3
//! Runtime6 / ordinary Invalid6 / stock-valid Unsupported21 / stock-invalid Unsupported12.
//! All original30 stop at the active allow-attribute boundary, not equality semantics.
//! Unsupported is the corpus category, not necessarily a native diagnostic error kind.
//! Native expectations passed in debug/release, with receipts retained under data/runs.
//! The existing associated_equality 46-case corpus case37_parameter_rhs
//! (SHA256 76fc73ffd704d98135cdc0e5fda7fcf590f8dfdcb9d39bf060803f87e36587c3)
//! is separately promoted to Runtime with its original source bytes unchanged.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/native_prelude.rs"]
mod native_prelude;

use justrust_engine::compiler::{self, frontend, ir};
use justrust_engine::{Key, Report};
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
    InvalidUnsupported,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    None,
    OrdinaryInvalid,
    Attribute,
    ReferenceRhs,
    CompositeRhs,
}
struct Case {
    name: &'static str,
    source: &'static str,
    source_bytes: usize,
    source_fingerprint: u64,
    stock_diagnostic: Option<&'static str>,
    stdout: &'static [u8],
    outcome: Outcome,
    phase: Phase,
    original_stock_flags: bool,
}
fn cases() -> Vec<Case> {
    vec![
        // Source SHA256 7c2e34d7e631b588e6ac954531bd069453be4015f5057fb0b371cbe9c890dbd8
        Case {
            name: "original14_scalar_pass",
            source: r#"#![allow(dead_code)]
trait Source: Sized { type Out; fn get(self) -> <Self as Source>::Out; }
struct Wrap<U>(U);
impl<U> Source for Wrap<U> { type Out = U; fn get(self) -> <Self as Source>::Out { self.0 } }
struct Token(i32);
fn pass<T,U>(x:T)->U where T:Source<Out=U> { <T as Source>::get(x) }
fn main(){ let v=pass::<Wrap<i32>,i32>(Wrap(41)); if v==41 { println!("ok"); } else { panic!("bad"); } }
"#,
            source_bytes: 400,
            source_fingerprint: 14967579562906360303,
            stock_diagnostic: None,
            stdout: b"ok\n",
            outcome: Outcome::Unsupported,
            phase: Phase::Attribute,
            original_stock_flags: true,
        },
        // Source SHA256 50d7afbbeb2f4a03b11d9145a58b1409aabefb5e4f1418a31a13e9b443431aa6
        Case {
            name: "original14_generic_wrap_out_u",
            source: r#"#![allow(dead_code)]
trait Source: Sized { type Out; fn get(self) -> <Self as Source>::Out; }
struct Wrap<U>(U);
impl<U> Source for Wrap<U> { type Out = U; fn get(self) -> <Self as Source>::Out { self.0 } }
struct Token(i32);
fn main(){ let v=<Wrap<Token> as Source>::get(Wrap(Token(42))); if v.0==42 { println!("ok"); } else { panic!("bad"); } }
"#,
            source_bytes: 347,
            source_fingerprint: 12162569970566463263,
            stock_diagnostic: None,
            stdout: b"ok\n",
            outcome: Outcome::Unsupported,
            phase: Phase::Attribute,
            original_stock_flags: true,
        },
        // Source SHA256 52ea60eb8e6a847a3bb85bcffe7485059d609484641e379db33a272e84575c4a
        Case {
            name: "original14_tuple_u_bool",
            source: r#"#![allow(dead_code)]
trait Source: Sized { type Out; fn get(self) -> <Self as Source>::Out; }
struct Wrap<U>(U);
impl<U> Source for Wrap<U> { type Out = U; fn get(self) -> <Self as Source>::Out { self.0 } }
struct Token(i32);
fn pair<T,U>(x:T)->(U,bool) where T:Source<Out=U> { (<T as Source>::get(x),true) }
fn main(){ let v=pair::<Wrap<Token>,Token>(Wrap(Token(43))); if v.0.0==43 && v.1 { println!("ok"); } else { panic!("bad"); } }
"#,
            source_bytes: 436,
            source_fingerprint: 14259107321289116128,
            stock_diagnostic: None,
            stdout: b"ok\n",
            outcome: Outcome::Unsupported,
            phase: Phase::Attribute,
            original_stock_flags: true,
        },
        // Source SHA256 5bec5cbcf6189fc1418513c5d6ea2b5f39dd8348bcd8bfb271a94f953817c2e1
        Case {
            name: "original14_noncopy_move_once",
            source: r#"#![allow(dead_code)]
trait Source: Sized { type Out; fn get(self) -> <Self as Source>::Out; }
struct Wrap<U>(U);
impl<U> Source for Wrap<U> { type Out = U; fn get(self) -> <Self as Source>::Out { self.0 } }
struct Token(i32);
fn pass<T,U>(x:T)->U where T:Source<Out=U> { <T as Source>::get(x) }
fn main(){ let a=pass::<Wrap<Token>,Token>(Wrap(Token(44))); let b=a; if b.0==44 { println!("ok"); } else { panic!("bad"); } }
"#,
            source_bytes: 422,
            source_fingerprint: 3042359064279682182,
            stock_diagnostic: None,
            stdout: b"ok\n",
            outcome: Outcome::Unsupported,
            phase: Phase::Attribute,
            original_stock_flags: true,
        },
        // Source SHA256 b373e323d2c0282806742165ef0ff3b017ee73f6de33757135bc9b1567e9d835
        Case {
            name: "original14_noncopy_duplicate",
            source: r#"#![allow(dead_code)]
trait Source: Sized { type Out; fn get(self) -> <Self as Source>::Out; }
struct Wrap<U>(U);
impl<U> Source for Wrap<U> { type Out = U; fn get(self) -> <Self as Source>::Out { self.0 } }
struct Token(i32);
fn twice<T,U>(x:T)->(U,U) where T:Source<Out=U> { let a:U=<T as Source>::get(x); (a,a) }
fn main(){ let v=twice::<Wrap<Token>,Token>(Wrap(Token(45))); if v.0.0==45 && v.1.0==45 { println!("ok"); } else { panic!("bad"); } }
"#,
            source_bytes: 449,
            source_fingerprint: 1004149603990002521,
            stock_diagnostic: Some("error[E0382]"),
            stdout: b"",
            outcome: Outcome::InvalidUnsupported,
            phase: Phase::Attribute,
            original_stock_flags: true,
        },
        // Source SHA256 c0771bde56019abd9349a19059c12c2d44128955014dbd6bf97beb7469902551
        Case {
            name: "original14_copy_duplicate",
            source: r#"#![allow(dead_code)]
trait Source: Sized { type Out; fn get(self) -> <Self as Source>::Out; }
struct Wrap<U>(U);
impl<U> Source for Wrap<U> { type Out = U; fn get(self) -> <Self as Source>::Out { self.0 } }
struct Token(i32);
fn twice<T,U:Copy>(x:T)->(U,U) where T:Source<Out=U> { let a:U=<T as Source>::get(x); (a,a) }
fn main(){ let v=twice::<Wrap<i32>,i32>(Wrap(46)); if v.0==46 && v.1==46 { println!("ok"); } else { panic!("bad"); } }
"#,
            source_bytes: 439,
            source_fingerprint: 1985193927613080579,
            stock_diagnostic: None,
            stdout: b"ok\n",
            outcome: Outcome::Unsupported,
            phase: Phase::Attribute,
            original_stock_flags: true,
        },
        // Source SHA256 26dcdfd17a996fe7b9b1240adca615242e567fde8c461a59890b8c9d448664c3
        Case {
            name: "original14_wrong_actual_equality",
            source: r#"#![allow(dead_code)]
trait Source: Sized { type Out; fn get(self) -> <Self as Source>::Out; }
struct Wrap<U>(U);
impl<U> Source for Wrap<U> { type Out = U; fn get(self) -> <Self as Source>::Out { self.0 } }
struct Token(i32);
fn pass<T,U>(x:T)->U where T:Source<Out=U> { <T as Source>::get(x) }
fn main(){ let v=pass::<Wrap<i32>,bool>(Wrap(47)); if v { println!("ok"); } else { panic!("bad"); } }
"#,
            source_bytes: 397,
            source_fingerprint: 13529973904423096008,
            stock_diagnostic: Some("error[E0271]"),
            stdout: b"",
            outcome: Outcome::InvalidUnsupported,
            phase: Phase::Attribute,
            original_stock_flags: true,
        },
        // Source SHA256 30f60524346f3f87492cdedbdf720b47196c2f470f5cb7591c43c0abb9dcdd06
        Case {
            name: "original14_unused_bad_generic_body",
            source: r#"#![allow(dead_code)]
trait Source: Sized { type Out; fn get(self) -> <Self as Source>::Out; }
struct Wrap<U>(U);
impl<U> Source for Wrap<U> { type Out = U; fn get(self) -> <Self as Source>::Out { self.0 } }
struct Token(i32);
fn bad<T,U>(x:T)->U where T:Source<Out=U> { let v:U=<T as Source>::get(x); let _:bool=v; v }
fn main(){ if 1==1 { println!("ok"); } else { panic!("bad"); } }
"#,
            source_bytes: 384,
            source_fingerprint: 3529922564033590143,
            stock_diagnostic: Some("error[E0308]"),
            stdout: b"",
            outcome: Outcome::InvalidUnsupported,
            phase: Phase::Attribute,
            original_stock_flags: true,
        },
        // Source SHA256 ced01836db8a3ff42d0ad226cc1ed9af2bcd5e90fbbfd6db5fccae578d2b6d70
        Case {
            name: "original14_foreign_binder_rhs",
            source: r#"#![allow(dead_code)]
trait Source: Sized { type Out; fn get(self) -> <Self as Source>::Out; }
struct Wrap<U>(U);
impl<U> Source for Wrap<U> { type Out = U; fn get(self) -> <Self as Source>::Out { self.0 } }
struct Token(i32);
fn foreign<V>(x:V)->V { x }
fn bad<T>(x:T)-><T as Source>::Out where T:Source<Out=V> { <T as Source>::get(x) }
fn main(){ if 1==1 { println!("ok"); } else { panic!("bad"); } }
"#,
            source_bytes: 402,
            source_fingerprint: 2466636380604489493,
            stock_diagnostic: Some("error[E0425]"),
            stdout: b"",
            outcome: Outcome::InvalidUnsupported,
            phase: Phase::Attribute,
            original_stock_flags: true,
        },
        // Source SHA256 1481fa37f7f95e69c4726b3893f90fc2b555a78054e25d92ea81d4cd04f5cc06
        Case {
            name: "original14_unbound_rhs",
            source: r#"#![allow(dead_code)]
trait Source: Sized { type Out; fn get(self) -> <Self as Source>::Out; }
struct Wrap<U>(U);
impl<U> Source for Wrap<U> { type Out = U; fn get(self) -> <Self as Source>::Out { self.0 } }
struct Token(i32);
fn bad<T>(x:T)-><T as Source>::Out where T:Source<Out=Missing> { <T as Source>::get(x) }
fn main(){ if 1==1 { println!("ok"); } else { panic!("bad"); } }
"#,
            source_bytes: 380,
            source_fingerprint: 3147052998172779545,
            stock_diagnostic: Some("error[E0425]"),
            stdout: b"",
            outcome: Outcome::InvalidUnsupported,
            phase: Phase::Attribute,
            original_stock_flags: true,
        },
        // Source SHA256 722dbfc1b2191512f6da3096436865c747a731996d1a00378546b2eab2f1bd66
        Case {
            name: "original14_reference_rhs_boundary",
            source: r#"#![allow(dead_code)]
trait Source: Sized { type Out; fn get(self) -> <Self as Source>::Out; }
struct Wrap<U>(U);
impl<U> Source for Wrap<U> { type Out = U; fn get(self) -> <Self as Source>::Out { self.0 } }
struct Token(i32);
fn reference<'a,T,U>(x:T)->&'a U where T:Source<Out=&'a U> { <T as Source>::get(x) }
fn main(){ let t=Token(48); let v=reference::<Wrap<&Token>,Token>(Wrap(&t)); if v.0==48 { println!("ok"); } else { panic!("bad"); } }
"#,
            source_bytes: 445,
            source_fingerprint: 12715146916765549008,
            stock_diagnostic: None,
            stdout: b"ok\n",
            outcome: Outcome::Unsupported,
            phase: Phase::Attribute,
            original_stock_flags: true,
        },
        // Source SHA256 a3bc8710cda3f2743fb65c3925227c49b46ba2f65ab7110cd8fef19718062201
        Case {
            name: "original14_typed_slice_reference_rhs_boundary",
            source: r#"#![allow(dead_code)]
trait Source: Sized { type Out; fn get(self) -> <Self as Source>::Out; }
struct Wrap<U>(U);
impl<U> Source for Wrap<U> { type Out = U; fn get(self) -> <Self as Source>::Out { self.0 } }
struct Token(i32);
fn slice<'a,T,U>(x:T)->&'a [U] where T:Source<Out=&'a [U]> { <T as Source>::get(x) }
fn main(){ let a=[49i32,50]; let s:&[i32]=&a; let v=slice::<Wrap<&[i32]>,i32>(Wrap(s)); if v[0]==49 && v[1]==50 { println!("ok"); } else { panic!("bad"); } }
"#,
            source_bytes: 469,
            source_fingerprint: 14441948406983991574,
            stock_diagnostic: None,
            stdout: b"ok\n",
            outcome: Outcome::Unsupported,
            phase: Phase::Attribute,
            original_stock_flags: true,
        },
        // Source SHA256 62aa2438f5416e22b0524af50d2d384e1fdf0214cc1c62e45fdb5805076d4604
        Case {
            name: "original14_method_pass_noncopy_external_u",
            source: r#"#![allow(dead_code)]
trait Source: Sized { type Out; fn pass(self,x:<Self as Source>::Out)-><Self as Source>::Out; }
struct Wrap<U>(U);
impl<U> Source for Wrap<U> { type Out=U; fn pass(self,x:<Self as Source>::Out)-><Self as Source>::Out { x } }
struct Token(i32);
fn forward<T,U>(owner:T,x:U)->U where T:Source<Out=U> { <T as Source>::pass(owner,x) }
fn main(){ let v=forward::<Wrap<Token>,Token>(Wrap(Token(99)),Token(51)); if v.0==51 { println!("ok"); } else { panic!("bad"); } }
"#,
            source_bytes: 483,
            source_fingerprint: 287462280172432221,
            stock_diagnostic: None,
            stdout: b"ok\n",
            outcome: Outcome::Unsupported,
            phase: Phase::Attribute,
            original_stock_flags: true,
        },
        // Source SHA256 34656a24a1c30cb28be6d68379f7784502eb76ad3e9e939fa42c88dcda959acc
        Case {
            name: "original14_method_pass_noncopy_duplicate",
            source: r#"#![allow(dead_code)]
trait Source: Sized { type Out; fn pass(self,x:<Self as Source>::Out)-><Self as Source>::Out; }
struct Wrap<U>(U);
impl<U> Source for Wrap<U> { type Out=U; fn pass(self,x:<Self as Source>::Out)-><Self as Source>::Out { x } }
struct Token(i32);
fn forward<T,U>(owner:T,x:U)->(U,U) where T:Source<Out=U> { let v=<T as Source>::pass(owner,x); (v,v) }
fn main(){ let v=forward::<Wrap<Token>,Token>(Wrap(Token(99)),Token(52)); if v.0.0==52 && v.1.0==52 { println!("ok"); } else { panic!("bad"); } }
"#,
            source_bytes: 515,
            source_fingerprint: 13425888558448884989,
            stock_diagnostic: Some("error[E0382]"),
            stdout: b"",
            outcome: Outcome::InvalidUnsupported,
            phase: Phase::Attribute,
            original_stock_flags: true,
        },
        // Source SHA256 998294cd494de913542e86122d54a795710d6592890f31514f7e88642a74acd5
        Case {
            name: "supplement1_tuple_rhs_u_bool",
            source: r#"#![allow(dead_code)]
trait Source: Sized { type Out; fn get(self)-><Self as Source>::Out; }
struct Wrap<U>(U);
impl<U> Source for Wrap<U> { type Out=(U,bool); fn get(self)-><Self as Source>::Out { (self.0,true) } }
struct Token(i32);
fn pair<T,U>(x:T)->(U,bool) where T:Source<Out=(U,bool)> { <T as Source>::get(x) }
fn main(){ let v=pair::<Wrap<Token>,Token>(Wrap(Token(53))); if v.0.0==53 && v.1 { println!("ok"); } else { panic!("bad"); } }
"#,
            source_bytes: 444,
            source_fingerprint: 18283035637400031816,
            stock_diagnostic: None,
            stdout: b"ok\n",
            outcome: Outcome::Unsupported,
            phase: Phase::Attribute,
            original_stock_flags: true,
        },
        // Source SHA256 f90b916e99120b57abf98c39f8272946e65e3dd58be11b9ccf5c380234b4fb38
        Case {
            name: "positive9_scalar_pass",
            source: r#"#![allow(dead_code)]
trait Source: Sized { type Out; fn get(self) -> <Self as Source>::Out; }
struct Wrap<U>(U);
impl<U> Source for Wrap<U> { type Out = U; fn get(self) -> <Self as Source>::Out { self.0 } }
struct Token(i32);
fn pass<T,U>(x:T)->U where T:Source<Out=U> { <T as Source>::get(x) }
fn main(){ let v=pass::<Wrap<i32>,i32>(Wrap(41)); if v==41 { println!("ok"); } }
"#,
            source_bytes: 376,
            source_fingerprint: 14406665776698823271,
            stock_diagnostic: None,
            stdout: b"ok\n",
            outcome: Outcome::Unsupported,
            phase: Phase::Attribute,
            original_stock_flags: false,
        },
        // Source SHA256 7a666b9f0b6391e4a9a10de4dbab321217fdd8942e02c79901a4ce0d09be1fb2
        Case {
            name: "positive9_generic_wrap_out_u",
            source: r#"#![allow(dead_code)]
trait Source: Sized { type Out; fn get(self) -> <Self as Source>::Out; }
struct Wrap<U>(U);
impl<U> Source for Wrap<U> { type Out = U; fn get(self) -> <Self as Source>::Out { self.0 } }
struct Token(i32);
fn main(){ let v=<Wrap<Token> as Source>::get(Wrap(Token(42))); if v.0==42 { println!("ok"); } }
"#,
            source_bytes: 323,
            source_fingerprint: 12323920166042596791,
            stock_diagnostic: None,
            stdout: b"ok\n",
            outcome: Outcome::Unsupported,
            phase: Phase::Attribute,
            original_stock_flags: false,
        },
        // Source SHA256 8fe442b2d099033e65303be9e776d1966ec3fcc906d2374a032b15a8e3586a64
        Case {
            name: "positive9_tuple_u_bool",
            source: r#"#![allow(dead_code)]
trait Source: Sized { type Out; fn get(self) -> <Self as Source>::Out; }
struct Wrap<U>(U);
impl<U> Source for Wrap<U> { type Out = U; fn get(self) -> <Self as Source>::Out { self.0 } }
struct Token(i32);
fn pair<T,U>(x:T)->(U,bool) where T:Source<Out=U> { (<T as Source>::get(x),true) }
fn main(){ let v=pair::<Wrap<Token>,Token>(Wrap(Token(43))); if v.0.0==43 && v.1 { println!("ok"); } }
"#,
            source_bytes: 412,
            source_fingerprint: 3015993875485395744,
            stock_diagnostic: None,
            stdout: b"ok\n",
            outcome: Outcome::Unsupported,
            phase: Phase::Attribute,
            original_stock_flags: false,
        },
        // Source SHA256 beb59c702e7c1b52898820e8b7ff57433bda19998d7c1794241d30d04990ad1a
        Case {
            name: "positive9_noncopy_move_once",
            source: r#"#![allow(dead_code)]
trait Source: Sized { type Out; fn get(self) -> <Self as Source>::Out; }
struct Wrap<U>(U);
impl<U> Source for Wrap<U> { type Out = U; fn get(self) -> <Self as Source>::Out { self.0 } }
struct Token(i32);
fn pass<T,U>(x:T)->U where T:Source<Out=U> { <T as Source>::get(x) }
fn main(){ let a=pass::<Wrap<Token>,Token>(Wrap(Token(44))); let b=a; if b.0==44 { println!("ok"); } }
"#,
            source_bytes: 398,
            source_fingerprint: 17277003457946407350,
            stock_diagnostic: None,
            stdout: b"ok\n",
            outcome: Outcome::Unsupported,
            phase: Phase::Attribute,
            original_stock_flags: false,
        },
        // Source SHA256 37c7eb29e45dc5785160d22a2e954b2d2585f9353f2bbe42d48a0f5560e8374e
        Case {
            name: "positive9_copy_duplicate",
            source: r#"#![allow(dead_code)]
trait Source: Sized { type Out; fn get(self) -> <Self as Source>::Out; }
struct Wrap<U>(U);
impl<U> Source for Wrap<U> { type Out = U; fn get(self) -> <Self as Source>::Out { self.0 } }
struct Token(i32);
fn twice<T,U:Copy>(x:T)->(U,U) where T:Source<Out=U> { let a:U=<T as Source>::get(x); (a,a) }
fn main(){ let v=twice::<Wrap<i32>,i32>(Wrap(46)); if v.0==46 && v.1==46 { println!("ok"); } }
"#,
            source_bytes: 415,
            source_fingerprint: 5315744930441834443,
            stock_diagnostic: None,
            stdout: b"ok\n",
            outcome: Outcome::Unsupported,
            phase: Phase::Attribute,
            original_stock_flags: false,
        },
        // Source SHA256 c9b96fceeee82e77269cbedc545c8caf6ffd77fcc23b72f0465bbf62745b999e
        Case {
            name: "positive9_reference_rhs_boundary",
            source: r#"#![allow(dead_code)]
trait Source: Sized { type Out; fn get(self) -> <Self as Source>::Out; }
struct Wrap<U>(U);
impl<U> Source for Wrap<U> { type Out = U; fn get(self) -> <Self as Source>::Out { self.0 } }
struct Token(i32);
fn reference<'a,T,U>(x:T)->&'a U where T:Source<Out=&'a U> { <T as Source>::get(x) }
fn main(){ let t=Token(48); let v=reference::<Wrap<&Token>,Token>(Wrap(&t)); if v.0==48 { println!("ok"); } }
"#,
            source_bytes: 421,
            source_fingerprint: 16678110351626800752,
            stock_diagnostic: None,
            stdout: b"ok\n",
            outcome: Outcome::Unsupported,
            phase: Phase::Attribute,
            original_stock_flags: false,
        },
        // Source SHA256 65f237dadc4362f2583daf139cff12f4247d9490fcecc98325265fdda87c893c
        Case {
            name: "positive9_typed_slice_reference_rhs_boundary",
            source: r#"#![allow(dead_code)]
trait Source: Sized { type Out; fn get(self) -> <Self as Source>::Out; }
struct Wrap<U>(U);
impl<U> Source for Wrap<U> { type Out = U; fn get(self) -> <Self as Source>::Out { self.0 } }
struct Token(i32);
fn slice<'a,T,U>(x:T)->&'a [U] where T:Source<Out=&'a [U]> { <T as Source>::get(x) }
fn main(){ let a=[49i32,50]; let s:&[i32]=&a; let v=slice::<Wrap<&[i32]>,i32>(Wrap(s)); if v[0]==49 && v[1]==50 { println!("ok"); } }
"#,
            source_bytes: 445,
            source_fingerprint: 7389467650796716646,
            stock_diagnostic: None,
            stdout: b"ok\n",
            outcome: Outcome::Unsupported,
            phase: Phase::Attribute,
            original_stock_flags: false,
        },
        // Source SHA256 a9deeaad96f322b5ac2364f8251fe7e32c8c3917984bb4007bfba1c09c2d88f0
        Case {
            name: "positive9_method_pass_noncopy_external_u",
            source: r#"#![allow(dead_code)]
trait Source: Sized { type Out; fn pass(self,x:<Self as Source>::Out)-><Self as Source>::Out; }
struct Wrap<U>(U);
impl<U> Source for Wrap<U> { type Out=U; fn pass(self,x:<Self as Source>::Out)-><Self as Source>::Out { x } }
struct Token(i32);
fn forward<T,U>(owner:T,x:U)->U where T:Source<Out=U> { <T as Source>::pass(owner,x) }
fn main(){ let v=forward::<Wrap<Token>,Token>(Wrap(Token(99)),Token(51)); if v.0==51 { println!("ok"); } }
"#,
            source_bytes: 459,
            source_fingerprint: 8652167284526130293,
            stock_diagnostic: None,
            stdout: b"ok\n",
            outcome: Outcome::Unsupported,
            phase: Phase::Attribute,
            original_stock_flags: false,
        },
        // Source SHA256 60fd771e2de7a58fcf54816e2bf17f6b9617ea34dc45c3b908b003d179400f0f
        Case {
            name: "positive9_tuple_rhs_u_bool",
            source: r#"#![allow(dead_code)]
trait Source: Sized { type Out; fn get(self)-><Self as Source>::Out; }
struct Wrap<U>(U);
impl<U> Source for Wrap<U> { type Out=(U,bool); fn get(self)-><Self as Source>::Out { (self.0,true) } }
struct Token(i32);
fn pair<T,U>(x:T)->(U,bool) where T:Source<Out=(U,bool)> { <T as Source>::get(x) }
fn main(){ let v=pair::<Wrap<Token>,Token>(Wrap(Token(53))); if v.0.0==53 && v.1 { println!("ok"); } }
"#,
            source_bytes: 420,
            source_fingerprint: 16461871791058706952,
            stock_diagnostic: None,
            stdout: b"ok\n",
            outcome: Outcome::Unsupported,
            phase: Phase::Attribute,
            original_stock_flags: false,
        },
        // Source SHA256 7c81900d16879ca7e40598c0024e251c0e2f73610b1013be6aede3399c1f5de8
        Case {
            name: "negative6_noncopy_duplicate",
            source: r#"#![allow(dead_code)]
trait Source: Sized { type Out; fn get(self) -> <Self as Source>::Out; }
struct Wrap<U>(U);
impl<U> Source for Wrap<U> { type Out = U; fn get(self) -> <Self as Source>::Out { self.0 } }
struct Token(i32);
fn twice<T,U>(x:T)->(U,U) where T:Source<Out=U> { let a:U=<T as Source>::get(x); (a,a) }
fn main(){ let v=twice::<Wrap<Token>,Token>(Wrap(Token(45))); if v.0.0==45 && v.1.0==45 { println!("ok"); } }
"#,
            source_bytes: 425,
            source_fingerprint: 10594397049083139233,
            stock_diagnostic: Some("error[E0382]"),
            stdout: b"",
            outcome: Outcome::InvalidUnsupported,
            phase: Phase::Attribute,
            original_stock_flags: false,
        },
        // Source SHA256 94b9cfae992af30ba19117ada1120ebcfd2597cc8d3dfe2cfd27e48f1322d316
        Case {
            name: "negative6_wrong_actual_equality",
            source: r#"#![allow(dead_code)]
trait Source: Sized { type Out; fn get(self) -> <Self as Source>::Out; }
struct Wrap<U>(U);
impl<U> Source for Wrap<U> { type Out = U; fn get(self) -> <Self as Source>::Out { self.0 } }
struct Token(i32);
fn pass<T,U>(x:T)->U where T:Source<Out=U> { <T as Source>::get(x) }
fn main(){ let v=pass::<Wrap<i32>,bool>(Wrap(47)); if v { println!("ok"); } }
"#,
            source_bytes: 373,
            source_fingerprint: 18262816097857859208,
            stock_diagnostic: Some("error[E0271]"),
            stdout: b"",
            outcome: Outcome::InvalidUnsupported,
            phase: Phase::Attribute,
            original_stock_flags: false,
        },
        // Source SHA256 4b660db4d6e696a41758a1c46e170679a914526c8ef2f644273e49c8b80cfaa2
        Case {
            name: "negative6_unused_bad_generic_body",
            source: r#"#![allow(dead_code)]
trait Source: Sized { type Out; fn get(self) -> <Self as Source>::Out; }
struct Wrap<U>(U);
impl<U> Source for Wrap<U> { type Out = U; fn get(self) -> <Self as Source>::Out { self.0 } }
struct Token(i32);
fn bad<T,U>(x:T)->U where T:Source<Out=U> { let v:U=<T as Source>::get(x); let _:bool=v; v }
fn main(){ if 1==1 { println!("ok"); } }
"#,
            source_bytes: 360,
            source_fingerprint: 1508269128863331223,
            stock_diagnostic: Some("error[E0308]"),
            stdout: b"",
            outcome: Outcome::InvalidUnsupported,
            phase: Phase::Attribute,
            original_stock_flags: false,
        },
        // Source SHA256 b19266e803d340b250e093537f04f1dd12e82fcc9b2444dbfc572881332121e3
        Case {
            name: "negative6_foreign_binder_rhs",
            source: r#"#![allow(dead_code)]
trait Source: Sized { type Out; fn get(self) -> <Self as Source>::Out; }
struct Wrap<U>(U);
impl<U> Source for Wrap<U> { type Out = U; fn get(self) -> <Self as Source>::Out { self.0 } }
struct Token(i32);
fn foreign<V>(x:V)->V { x }
fn bad<T>(x:T)-><T as Source>::Out where T:Source<Out=V> { <T as Source>::get(x) }
fn main(){ if 1==1 { println!("ok"); } }
"#,
            source_bytes: 378,
            source_fingerprint: 1017437650980827725,
            stock_diagnostic: Some("error[E0425]"),
            stdout: b"",
            outcome: Outcome::InvalidUnsupported,
            phase: Phase::Attribute,
            original_stock_flags: false,
        },
        // Source SHA256 4b177214d47c8db3da71e39036b5bee748b952ff68f951d0c05f891aaa30d612
        Case {
            name: "negative6_unbound_rhs",
            source: r#"#![allow(dead_code)]
trait Source: Sized { type Out; fn get(self) -> <Self as Source>::Out; }
struct Wrap<U>(U);
impl<U> Source for Wrap<U> { type Out = U; fn get(self) -> <Self as Source>::Out { self.0 } }
struct Token(i32);
fn bad<T>(x:T)-><T as Source>::Out where T:Source<Out=Missing> { <T as Source>::get(x) }
fn main(){ if 1==1 { println!("ok"); } }
"#,
            source_bytes: 356,
            source_fingerprint: 11868097146663022689,
            stock_diagnostic: Some("error[E0425]"),
            stdout: b"",
            outcome: Outcome::InvalidUnsupported,
            phase: Phase::Attribute,
            original_stock_flags: false,
        },
        // Source SHA256 b0d158532fe485217de610accd2b40b773caec9231225a29e22c9b67701bb9bb
        Case {
            name: "negative6_method_pass_noncopy_duplicate",
            source: r#"#![allow(dead_code)]
trait Source: Sized { type Out; fn pass(self,x:<Self as Source>::Out)-><Self as Source>::Out; }
struct Wrap<U>(U);
impl<U> Source for Wrap<U> { type Out=U; fn pass(self,x:<Self as Source>::Out)-><Self as Source>::Out { x } }
struct Token(i32);
fn forward<T,U>(owner:T,x:U)->(U,U) where T:Source<Out=U> { let v=<T as Source>::pass(owner,x); (v,v) }
fn main(){ let v=forward::<Wrap<Token>,Token>(Wrap(Token(99)),Token(52)); if v.0.0==52 && v.1.0==52 { println!("ok"); } }
"#,
            source_bytes: 491,
            source_fingerprint: 8588926474408597397,
            stock_diagnostic: Some("error[E0382]"),
            stdout: b"",
            outcome: Outcome::InvalidUnsupported,
            phase: Phase::Attribute,
            original_stock_flags: false,
        },
        // Source SHA256 da57bf988e5e8d2868040055a2f8fa3e140856a5fdd378ca0562640a34bc4eb2
        Case {
            name: "stripped_positive9_scalar_pass",
            source: r#"trait Source: Sized { type Out; fn get(self) -> <Self as Source>::Out; }
struct Wrap<U>(U);
impl<U> Source for Wrap<U> { type Out = U; fn get(self) -> <Self as Source>::Out { self.0 } }
struct Token(i32);
fn pass<T,U>(x:T)->U where T:Source<Out=U> { <T as Source>::get(x) }
fn main(){ let v=pass::<Wrap<i32>,i32>(Wrap(41)); if v==41 { println!("ok"); } }
"#,
            source_bytes: 355,
            source_fingerprint: 15041068264127761645,
            stock_diagnostic: None,
            stdout: b"ok\n",
            outcome: Outcome::Runtime,
            phase: Phase::None,
            original_stock_flags: false,
        },
        // Source SHA256 2d537628897f3f6efe241eb23bbb52426ba87ce40dfca7292899feccf34c4c68
        Case {
            name: "stripped_positive9_generic_wrap_out_u",
            source: r#"trait Source: Sized { type Out; fn get(self) -> <Self as Source>::Out; }
struct Wrap<U>(U);
impl<U> Source for Wrap<U> { type Out = U; fn get(self) -> <Self as Source>::Out { self.0 } }
struct Token(i32);
fn main(){ let v=<Wrap<Token> as Source>::get(Wrap(Token(42))); if v.0==42 { println!("ok"); } }
"#,
            source_bytes: 302,
            source_fingerprint: 3869403722723434681,
            stock_diagnostic: None,
            stdout: b"ok\n",
            outcome: Outcome::Runtime,
            phase: Phase::None,
            original_stock_flags: false,
        },
        // Source SHA256 ffb505ba59c26329ff4b025ac1bce0870f6a3f755ef1a65da2788ff252b2c773
        Case {
            name: "stripped_positive9_tuple_u_bool",
            source: r#"trait Source: Sized { type Out; fn get(self) -> <Self as Source>::Out; }
struct Wrap<U>(U);
impl<U> Source for Wrap<U> { type Out = U; fn get(self) -> <Self as Source>::Out { self.0 } }
struct Token(i32);
fn pair<T,U>(x:T)->(U,bool) where T:Source<Out=U> { (<T as Source>::get(x),true) }
fn main(){ let v=pair::<Wrap<Token>,Token>(Wrap(Token(43))); if v.0.0==43 && v.1 { println!("ok"); } }
"#,
            source_bytes: 391,
            source_fingerprint: 16055129404165104306,
            stock_diagnostic: None,
            stdout: b"ok\n",
            outcome: Outcome::Runtime,
            phase: Phase::None,
            original_stock_flags: false,
        },
        // Source SHA256 fb21bdae9bf03bf36ecebfe2cd5a84df43a9594edd02a4c8045183e617778df8
        Case {
            name: "stripped_positive9_noncopy_move_once",
            source: r#"trait Source: Sized { type Out; fn get(self) -> <Self as Source>::Out; }
struct Wrap<U>(U);
impl<U> Source for Wrap<U> { type Out = U; fn get(self) -> <Self as Source>::Out { self.0 } }
struct Token(i32);
fn pass<T,U>(x:T)->U where T:Source<Out=U> { <T as Source>::get(x) }
fn main(){ let a=pass::<Wrap<Token>,Token>(Wrap(Token(44))); let b=a; if b.0==44 { println!("ok"); } }
"#,
            source_bytes: 377,
            source_fingerprint: 11694806929102525048,
            stock_diagnostic: None,
            stdout: b"ok\n",
            outcome: Outcome::Runtime,
            phase: Phase::None,
            original_stock_flags: false,
        },
        // Source SHA256 776ab1f643784a1b2b3b3b49c9821f5c50d4a459c1db3567f515c0597b905532
        Case {
            name: "stripped_positive9_copy_duplicate",
            source: r#"trait Source: Sized { type Out; fn get(self) -> <Self as Source>::Out; }
struct Wrap<U>(U);
impl<U> Source for Wrap<U> { type Out = U; fn get(self) -> <Self as Source>::Out { self.0 } }
struct Token(i32);
fn twice<T,U:Copy>(x:T)->(U,U) where T:Source<Out=U> { let a:U=<T as Source>::get(x); (a,a) }
fn main(){ let v=twice::<Wrap<i32>,i32>(Wrap(46)); if v.0==46 && v.1==46 { println!("ok"); } }
"#,
            source_bytes: 394,
            source_fingerprint: 7090780752219289449,
            stock_diagnostic: None,
            stdout: b"ok\n",
            outcome: Outcome::Runtime,
            phase: Phase::None,
            original_stock_flags: false,
        },
        // Source SHA256 a6849d991fdf165a290dd7eb35a5ca4612d60a6c236b2030a28b667f2eb66055
        Case {
            name: "stripped_positive9_reference_rhs_boundary",
            source: r#"trait Source: Sized { type Out; fn get(self) -> <Self as Source>::Out; }
struct Wrap<U>(U);
impl<U> Source for Wrap<U> { type Out = U; fn get(self) -> <Self as Source>::Out { self.0 } }
struct Token(i32);
fn reference<'a,T,U>(x:T)->&'a U where T:Source<Out=&'a U> { <T as Source>::get(x) }
fn main(){ let t=Token(48); let v=reference::<Wrap<&Token>,Token>(Wrap(&t)); if v.0==48 { println!("ok"); } }
"#,
            source_bytes: 400,
            source_fingerprint: 13518186116290668066,
            stock_diagnostic: None,
            stdout: b"ok\n",
            outcome: Outcome::Unsupported,
            phase: Phase::ReferenceRhs,
            original_stock_flags: false,
        },
        // Source SHA256 4c452c56820cc2cdc97df625f2f4716f76cc7e9b2b194c1a6486f82aaa91c173
        Case {
            name: "stripped_positive9_typed_slice_reference_rhs_boundary",
            source: r#"trait Source: Sized { type Out; fn get(self) -> <Self as Source>::Out; }
struct Wrap<U>(U);
impl<U> Source for Wrap<U> { type Out = U; fn get(self) -> <Self as Source>::Out { self.0 } }
struct Token(i32);
fn slice<'a,T,U>(x:T)->&'a [U] where T:Source<Out=&'a [U]> { <T as Source>::get(x) }
fn main(){ let a=[49i32,50]; let s:&[i32]=&a; let v=slice::<Wrap<&[i32]>,i32>(Wrap(s)); if v[0]==49 && v[1]==50 { println!("ok"); } }
"#,
            source_bytes: 424,
            source_fingerprint: 10627086874618894260,
            stock_diagnostic: None,
            stdout: b"ok\n",
            outcome: Outcome::Unsupported,
            phase: Phase::CompositeRhs,
            original_stock_flags: false,
        },
        // Source SHA256 6bbed7fe5efa0f88cf3640dd76f13a597c66fdad4e3d9d2c5d891c0ece041091
        Case {
            name: "stripped_positive9_method_pass_noncopy_external_u",
            source: r#"trait Source: Sized { type Out; fn pass(self,x:<Self as Source>::Out)-><Self as Source>::Out; }
struct Wrap<U>(U);
impl<U> Source for Wrap<U> { type Out=U; fn pass(self,x:<Self as Source>::Out)-><Self as Source>::Out { x } }
struct Token(i32);
fn forward<T,U>(owner:T,x:U)->U where T:Source<Out=U> { <T as Source>::pass(owner,x) }
fn main(){ let v=forward::<Wrap<Token>,Token>(Wrap(Token(99)),Token(51)); if v.0==51 { println!("ok"); } }
"#,
            source_bytes: 438,
            source_fingerprint: 8700600147641175471,
            stock_diagnostic: None,
            stdout: b"ok\n",
            outcome: Outcome::Runtime,
            phase: Phase::None,
            original_stock_flags: false,
        },
        // Source SHA256 16c797b3399f4b580ab2e1feeecd0f33acb2380241bd3e871d29881bfaa40140
        Case {
            name: "stripped_positive9_tuple_rhs_u_bool",
            source: r#"trait Source: Sized { type Out; fn get(self)-><Self as Source>::Out; }
struct Wrap<U>(U);
impl<U> Source for Wrap<U> { type Out=(U,bool); fn get(self)-><Self as Source>::Out { (self.0,true) } }
struct Token(i32);
fn pair<T,U>(x:T)->(U,bool) where T:Source<Out=(U,bool)> { <T as Source>::get(x) }
fn main(){ let v=pair::<Wrap<Token>,Token>(Wrap(Token(53))); if v.0.0==53 && v.1 { println!("ok"); } }
"#,
            source_bytes: 399,
            source_fingerprint: 15388969938197766946,
            stock_diagnostic: None,
            stdout: b"ok\n",
            outcome: Outcome::Unsupported,
            phase: Phase::CompositeRhs,
            original_stock_flags: false,
        },
        // Source SHA256 fb21f9d842a12550759d656301db196cdee61024af80227538a862ce157f7ef6
        Case {
            name: "stripped_negative6_noncopy_duplicate",
            source: r#"trait Source: Sized { type Out; fn get(self) -> <Self as Source>::Out; }
struct Wrap<U>(U);
impl<U> Source for Wrap<U> { type Out = U; fn get(self) -> <Self as Source>::Out { self.0 } }
struct Token(i32);
fn twice<T,U>(x:T)->(U,U) where T:Source<Out=U> { let a:U=<T as Source>::get(x); (a,a) }
fn main(){ let v=twice::<Wrap<Token>,Token>(Wrap(Token(45))); if v.0.0==45 && v.1.0==45 { println!("ok"); } }
"#,
            source_bytes: 404,
            source_fingerprint: 5260562235977620915,
            stock_diagnostic: Some("error[E0382]"),
            stdout: b"",
            outcome: Outcome::Invalid,
            phase: Phase::OrdinaryInvalid,
            original_stock_flags: false,
        },
        // Source SHA256 3a8662b9a1c0bc5634b33f48f63d8c8230d317330c9d5b282d9b964073082d24
        Case {
            name: "stripped_negative6_wrong_actual_equality",
            source: r#"trait Source: Sized { type Out; fn get(self) -> <Self as Source>::Out; }
struct Wrap<U>(U);
impl<U> Source for Wrap<U> { type Out = U; fn get(self) -> <Self as Source>::Out { self.0 } }
struct Token(i32);
fn pass<T,U>(x:T)->U where T:Source<Out=U> { <T as Source>::get(x) }
fn main(){ let v=pass::<Wrap<i32>,bool>(Wrap(47)); if v { println!("ok"); } }
"#,
            source_bytes: 352,
            source_fingerprint: 3571155748059623970,
            stock_diagnostic: Some("error[E0271]"),
            stdout: b"",
            outcome: Outcome::Invalid,
            phase: Phase::OrdinaryInvalid,
            original_stock_flags: false,
        },
        // Source SHA256 de503c8d1b5fb1b65996fa8bec84f6fb8f5fa6052ffbad80ccf396a4f5f31d0b
        Case {
            name: "stripped_negative6_unused_bad_generic_body",
            source: r#"trait Source: Sized { type Out; fn get(self) -> <Self as Source>::Out; }
struct Wrap<U>(U);
impl<U> Source for Wrap<U> { type Out = U; fn get(self) -> <Self as Source>::Out { self.0 } }
struct Token(i32);
fn bad<T,U>(x:T)->U where T:Source<Out=U> { let v:U=<T as Source>::get(x); let _:bool=v; v }
fn main(){ if 1==1 { println!("ok"); } }
"#,
            source_bytes: 339,
            source_fingerprint: 1958429043784757609,
            stock_diagnostic: Some("error[E0308]"),
            stdout: b"",
            outcome: Outcome::Invalid,
            phase: Phase::OrdinaryInvalid,
            original_stock_flags: false,
        },
        // Source SHA256 cbddb37fb9376f07189b213a6b92905b1afee39c74f1ee7873233889ea48bb88
        Case {
            name: "stripped_negative6_foreign_binder_rhs",
            source: r#"trait Source: Sized { type Out; fn get(self) -> <Self as Source>::Out; }
struct Wrap<U>(U);
impl<U> Source for Wrap<U> { type Out = U; fn get(self) -> <Self as Source>::Out { self.0 } }
struct Token(i32);
fn foreign<V>(x:V)->V { x }
fn bad<T>(x:T)-><T as Source>::Out where T:Source<Out=V> { <T as Source>::get(x) }
fn main(){ if 1==1 { println!("ok"); } }
"#,
            source_bytes: 357,
            source_fingerprint: 10334386092890505671,
            stock_diagnostic: Some("error[E0425]"),
            stdout: b"",
            outcome: Outcome::Invalid,
            phase: Phase::OrdinaryInvalid,
            original_stock_flags: false,
        },
        // Source SHA256 c0d434ccb68fe943996e5e821fa89146183aa48d34bc9e8ffa5b287b9438b0ae
        Case {
            name: "stripped_negative6_unbound_rhs",
            source: r#"trait Source: Sized { type Out; fn get(self) -> <Self as Source>::Out; }
struct Wrap<U>(U);
impl<U> Source for Wrap<U> { type Out = U; fn get(self) -> <Self as Source>::Out { self.0 } }
struct Token(i32);
fn bad<T>(x:T)-><T as Source>::Out where T:Source<Out=Missing> { <T as Source>::get(x) }
fn main(){ if 1==1 { println!("ok"); } }
"#,
            source_bytes: 335,
            source_fingerprint: 7588750460277362731,
            stock_diagnostic: Some("error[E0425]"),
            stdout: b"",
            outcome: Outcome::Invalid,
            phase: Phase::OrdinaryInvalid,
            original_stock_flags: false,
        },
        // Source SHA256 cae4e83af4f70e81ed606f443d180ade7a9586521fe18f3ac20348ad8fc7ac9d
        Case {
            name: "stripped_negative6_method_pass_noncopy_duplicate",
            source: r#"trait Source: Sized { type Out; fn pass(self,x:<Self as Source>::Out)-><Self as Source>::Out; }
struct Wrap<U>(U);
impl<U> Source for Wrap<U> { type Out=U; fn pass(self,x:<Self as Source>::Out)-><Self as Source>::Out { x } }
struct Token(i32);
fn forward<T,U>(owner:T,x:U)->(U,U) where T:Source<Out=U> { let v=<T as Source>::pass(owner,x); (v,v) }
fn main(){ let v=forward::<Wrap<Token>,Token>(Wrap(Token(99)),Token(52)); if v.0.0==52 && v.1.0==52 { println!("ok"); } }
"#,
            source_bytes: 470,
            source_fingerprint: 8715134495258331683,
            stock_diagnostic: Some("error[E0382]"),
            stdout: b"",
            outcome: Outcome::Invalid,
            phase: Phase::OrdinaryInvalid,
            original_stock_flags: false,
        },
    ]
}
/// Native expectation. The active `allow` attribute is now supported, so the
/// 30 attributed cases follow stock semantics: stock-invalid sources are
/// ordinary Invalid, reference-valued equality RHS stays Unsupported, and
/// every other source runs with stock's stdout.
fn native(case: &Case) -> (Outcome, Phase) {
    if case.name.ends_with("reference_rhs_boundary") {
        let phase = if case.name.contains("typed_slice") {
            Phase::CompositeRhs
        } else {
            Phase::ReferenceRhs
        };
        return (Outcome::Unsupported, phase);
    }
    if case.stock_diagnostic.is_some() {
        return (Outcome::Invalid, Phase::OrdinaryInvalid);
    }
    (Outcome::Runtime, Phase::None)
}
/// Native cells per [Runtime, Invalid, Unsupported, InvalidUnsupported] x [CHECK, BUILD]:
/// 21 Runtime, 18 Invalid, 6 reference-RHS Unsupported cases, 4 cells each.
const NATIVE_MODES: [[usize; 2]; 4] = [[42, 42], [36, 36], [12, 12], [0, 0]];
fn partition_index(outcome: Outcome) -> usize {
    match outcome {
        Outcome::Runtime => 0,
        Outcome::Invalid => 1,
        Outcome::Unsupported => 2,
        Outcome::InvalidUnsupported => 3,
    }
}
fn fingerprint(source: &str) -> u64 {
    source.bytes().fold(14695981039346656037, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(1099511628211)
    })
}
#[test]
fn parameter_equality_frozen_stock_partition_and_source_bytes() {
    let cases = cases();
    assert_eq!(cases.len(), 45);
    let mut names = std::collections::BTreeSet::new();
    let mut partition = [0; 4];
    let mut stock = [0; 2];
    for (index, case) in cases.iter().enumerate() {
        assert!(names.insert(case.name), "duplicate name {}", case.name);
        let prefix = if index < 14 {
            "original14_"
        } else if index == 14 {
            "supplement1_"
        } else if index < 24 {
            "positive9_"
        } else if index < 30 {
            "negative6_"
        } else {
            "stripped_"
        };
        assert!(case.name.starts_with(prefix), "{}", case.name);
        assert_eq!(case.source.len(), case.source_bytes, "{}", case.name);
        assert_eq!(
            fingerprint(case.source),
            case.source_fingerprint,
            "{}",
            case.name
        );
        assert!(case.source.ends_with('\n'));
        partition[partition_index(case.outcome)] += 1;
        stock[usize::from(case.stock_diagnostic.is_some())] += 1;
        assert_eq!(
            case.stock_diagnostic.is_some(),
            matches!(case.outcome, Outcome::Invalid | Outcome::InvalidUnsupported)
        );
        assert_eq!(case.phase == Phase::None, case.outcome == Outcome::Runtime);
        assert_eq!(case.source.contains("panic!("), index < 15);
        assert_eq!(case.phase == Phase::Attribute, index < 30);
        assert_eq!(
            case.source.starts_with("#![allow(dead_code)]\n"),
            index < 30
        );
    }
    let ordered: String = cases[..30].iter().map(|case| case.source).collect();
    assert_eq!(fingerprint(&ordered), 18208294126689561605);
    let appended: String = cases[30..].iter().map(|case| case.source).collect();
    assert_eq!(fingerprint(&appended), 7734481242093918221);
    for (batch, expected) in [(&cases[..30], [0, 0, 18, 12]), (&cases[30..], [6, 6, 3, 0])] {
        let mut actual = [0; 4];
        for case in batch {
            actual[partition_index(case.outcome)] += 1;
        }
        assert_eq!(actual, expected);
    }
    assert_eq!(partition, [6, 6, 21, 12]);
    assert_eq!(stock, [27, 18]);
    for (parent, stripped) in cases[15..30].iter().zip(&cases[30..]) {
        assert_eq!(
            stripped.source,
            parent
                .source
                .strip_prefix("#![allow(dead_code)]\n")
                .unwrap()
        );
        assert_eq!(stripped.stock_diagnostic, parent.stock_diagnostic);
        assert_eq!(stripped.stdout, parent.stdout);
    }
    for negative in &cases[24..30] {
        let original = cases[..14]
            .iter()
            .find(|c| {
                c.name.strip_prefix("original14_") == negative.name.strip_prefix("negative6_")
            })
            .unwrap();
        assert_eq!(original.stock_diagnostic, negative.stock_diagnostic);
        assert_eq!(
            original.source.replace(" else { panic!(\"bad\"); }", ""),
            negative.source
        );
    }
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
            "jr-parameter-equality-{}-{nonce}-{}",
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
    capture_bounded(command, scratch, seconds, 65536)
}
fn capture_bounded(
    command: &mut Command,
    scratch: &Scratch,
    seconds: u64,
    output_limit: u64,
) -> Captured {
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
                fs::metadata(path).unwrap().len() <= output_limit,
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
        assert!(fs::metadata(path).unwrap().len() <= output_limit);
    }
    Captured {
        code: status.code().expect("signal termination"),
        stdout: fs::read(out).unwrap(),
        stderr: fs::read(err).unwrap(),
    }
}
fn runtime(path: &Path, scratch: &Scratch, expected: &[u8]) {
    let image = fs::read(path).unwrap();
    assert!(image.len() >= 64);
    assert_eq!(&image[..6], b"\x7fELF\x02\x01");
    let result = capture(&mut Command::new(path), scratch, 5);
    assert_eq!(result.code, 0, "runtime: {result:?}");
    assert_eq!(result.stdout, expected);
    assert!(result.stderr.is_empty(), "{result:?}");
}
fn no_compiler_tools(command: &mut Command, scratch: &Scratch) {
    command
        .current_dir(&scratch.0)
        .env_remove("RUSTC_BOOTSTRAP")
        .env("RUSTC", scratch.path("no-rustc"))
        .env("PATH", scratch.path("no-tools"));
}

fn assert_native_diagnostic(case: &Case, diagnostic: &str) {
    assert!(!diagnostic.is_empty(), "{}: missing diagnostic", case.name);
    assert!(
        !diagnostic.contains("panicked at"),
        "{}:{diagnostic}",
        case.name
    );
    let (outcome, phase) = native(case);
    let expected = match phase {
        Phase::Attribute => "active attribute `allow` is unsupported",
        Phase::ReferenceRhs | Phase::CompositeRhs => {
            "unsupported: equality RHS must be closed or one authoritative parameter"
        }
        Phase::OrdinaryInvalid => {
            assert_eq!(outcome, Outcome::Invalid);
            assert!(
                !diagnostic.contains("unsupported:"),
                "ordinary invalid became Unsupported: {}:{diagnostic}",
                case.name
            );
            return;
        }
        Phase::None => panic!("unexpected rejection of {}:{diagnostic}", case.name),
    };
    assert!(
        diagnostic.contains(expected),
        "{} expected {:?}, {expected:?}: {diagnostic}",
        case.name,
        phase
    );
}
// Catch at cell boundaries, never suppress individual assertions or process safety guards.
// Every failed cell is printed with its source and mode before the aggregate gate fails.
#[derive(Default)]
struct CellFailures(Vec<String>);
impl CellFailures {
    fn check(&mut self, label: impl Into<String>, check: impl FnOnce()) {
        let label = label.into();
        if let Err(payload) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(check)) {
            let message = payload
                .downcast_ref::<String>()
                .map(String::as_str)
                .or_else(|| payload.downcast_ref::<&str>().copied())
                .unwrap_or("non-string panic payload");
            let failure = format!("{label}: {message}");
            eprintln!("PARAMETER EQUALITY CELL FAILURE: {failure}");
            self.0.push(failure);
        } else {
            eprintln!("PARAMETER EQUALITY CELL PASS: {label}");
        }
    }
    fn finish(self) {
        assert!(
            self.0.is_empty(),
            "{} parameter equality failures:\n{}",
            self.0.len(),
            self.0.join("\n")
        );
    }
}
fn stock_all() {
    let scratch = Scratch::new();
    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let version = capture(
        Command::new(&rustc)
            .env_remove("RUSTC_BOOTSTRAP")
            .arg("-vV"),
        &scratch,
        10,
    );
    assert_eq!(version.code, 0, "{version:?}");
    // Frozen measurements pin the exact baseline. Replays also record their actual compiler.
    eprintln!(
        "PARAMETER EQUALITY FROZEN BASELINE:{BASELINE_STOCK}\nACTUAL STOCK:\n{}",
        String::from_utf8_lossy(&version.stdout)
    );
    let mut counts = [0; 2];
    let mut failures = CellFailures::default();
    for case in cases() {
        counts[usize::from(case.stock_diagnostic.is_some())] += 1;
        failures.check(format!("stock {}", case.name), || {
            let scratch = Scratch::new();
            scratch.write("client.rs", case.source.as_bytes());
            let image = scratch.path("stock");
            let result = capture(
                Command::new(&rustc)
                    .env_remove("RUSTC_BOOTSTRAP")
                    .args([
                        if case.original_stock_flags {
                            "--edition=2021"
                        } else {
                            "--edition=2024"
                        },
                        "--crate-name=parameter_equality_oracle",
                        "--color=never",
                    ])
                    .args(if case.original_stock_flags {
                        Vec::<&str>::new()
                    } else {
                        vec!["-Cpanic=abort"]
                    })
                    .arg(scratch.path("client.rs"))
                    .arg("-o")
                    .arg(&image),
                &scratch,
                20,
            );
            assert!(result.stdout.is_empty(), "{}:{result:?}", case.name);
            if let Some(expected) = case.stock_diagnostic {
                assert_eq!(result.code, 1, "{}:{result:?}", case.name);
                assert!(
                    String::from_utf8_lossy(&result.stderr).contains(expected),
                    "{}:{result:?}",
                    case.name
                );
                let stderr = String::from_utf8_lossy(&result.stderr);
                let categories: Vec<_> = stderr
                    .lines()
                    .filter(|line| line.starts_with("error[E"))
                    .map(|line| line.split(':').next().unwrap())
                    .collect();
                assert_eq!(categories, [expected], "{}:{result:?}", case.name);
                assert!(!image.exists());
            } else {
                assert_eq!(result.code, 0, "{}:{result:?}", case.name);
                runtime(&image, &scratch, case.stdout);
            }
        });
    }
    assert_eq!(counts, [27, 18]);
    failures.finish();
    eprintln!(
        "STOCK PARAMETER EQUALITY:45 frozen sources,27 accepted with exact payload,18 rejected; original30+attribute-stripped15"
    );
}
#[test]
fn stock_parameter_equality_oracle_frozen_editions_actual_toolchain() {
    stock_all();
}
#[test]
fn native_parameter_equality_workers1_8_check_build_no_compiler_fallback() {
    // Aggregate a stock failure without preventing native coverage.
    let mut failures = CellFailures::default();
    failures.check("stock prerequisite", stock_all);
    let mut cells = 0;
    let mut modes = [[0; 2]; 4];
    let mut runtimes = 0;
    for case in cases() {
        let (outcome, phase) = native(&case);
        for workers in [1, 8] {
            for check in [true, false] {
                failures.check(
                    format!(
                        "native CLI {} workers{workers} check{check} expected {outcome:?}/{phase:?}",
                        case.name
                    ),
                    || {
                        let scratch = Scratch::new();
                        scratch.write("client.rs", case.source.as_bytes());
                        let image = scratch.path("native");
                        let mut command = Command::new(env!("CARGO_BIN_EXE_jr-rust"));
                        no_compiler_tools(&mut command, &scratch);
                        command
                            .arg(scratch.path("client.rs"))
                            .args(["--workers", &workers.to_string()]);
                        if check {
                            command.arg("--check");
                        } else {
                            command.arg("-o").arg(&image);
                        }
                        let result = capture(&mut command, &scratch, 20);
                        assert_eq!(
                            result.code,
                            i32::from(outcome != Outcome::Runtime),
                            "{} workers{workers} check{check}:{result:?}",
                            case.name
                        );
                        if outcome != Outcome::Runtime {
                            assert!(result.stdout.is_empty(), "{}:{result:?}", case.name);
                            assert_native_diagnostic(
                                &case,
                                &String::from_utf8_lossy(&result.stderr),
                            );
                        }
                        if !check && outcome == Outcome::Runtime {
                            runtime(&image, &scratch, case.stdout);
                            runtimes += 1;
                        } else {
                            for path in ["native", "client", "a.out"] {
                                assert!(
                                    !scratch.path(path).exists(),
                                    "artifact {path}:{}",
                                    case.name
                                );
                            }
                        }
                    },
                );
                modes[partition_index(outcome)][usize::from(!check)] += 1;
                cells += 1;
            }
        }
    }
    assert_eq!(cells, 180);
    assert_eq!(modes, NATIVE_MODES);
    failures.check("native CLI successful runtime count", || {
        assert_eq!(runtimes, NATIVE_MODES[0][1])
    });
    failures.finish();
    eprintln!(
        "NATIVE PARAMETER EQUALITY:180 CLI cells,modes{modes:?},{runtimes} native ELF payloads"
    );
}
fn evaluated_once(report: &Report, kind: u32, index: usize, case: &Case) {
    let key = Key::new(kind, vec![index as u64]);
    let entries: Vec<_> = report
        .node_evaluations
        .iter()
        .filter(|(actual, _)| *actual == key)
        .collect();
    assert_eq!(entries.len(), 1, "{}: missing/duplicate {key:?}", case.name);
    assert_eq!(entries[0].1, 1, "{}: repeated {key:?}", case.name);
}
fn successful_all_body_graph(case: &Case, compiled: &compiler::Compilation, check: bool) {
    let program = native_prelude::parse(case.source).expect("frozen positive source parses");
    frontend::check(&program).expect("all bodies check independently of reachability");
    let plan = frontend::PlanCtx::from_program(&program, &ir::PlanBudget::default())
        .expect("real source binders and declared predicates");
    assert_eq!(compiled.source_bytes, case.source.len());
    assert_eq!(compiled.source_files, 1);
    assert_eq!(compiled.executable.is_none(), check);
    assert_eq!(compiled.report.failures, 0);
    assert_eq!(compiled.report.events_dropped, 0);
    assert!(compiled.report.capture_complete);
    let mut eager_bodies = 0usize;
    let mut source_bodies = 0;
    let mut generic_bodies = 0;
    for index in 0..plan.callable_count() {
        let declaration = plan.callable_decl(index).unwrap();
        // Embedded prelude bodies are admitted on demand, never eagerly.
        if plan.is_prelude_decl(declaration).unwrap() {
            continue;
        }
        eager_bodies += 1;
        evaluated_once(&compiled.report, 110, index, case);
        let universal = match plan.callable_provider(declaration).unwrap() {
            frontend::CallableProvider::Source { .. } => {
                source_bodies += usize::from(!plan.is_prelude_decl(declaration).unwrap());
                let generic = plan.callable_env(declaration).unwrap().is_generic();
                generic_bodies += usize::from(generic);
                generic
            }
            frontend::CallableProvider::Recipe { .. } => true,
            frontend::CallableProvider::UnavailableProvided { .. } => {
                panic!("{}: unavailable body entered inventory", case.name)
            }
        };
        if universal {
            evaluated_once(&compiled.report, 111, index, case);
            evaluated_once(&compiled.report, 112, index, case);
        }
    }
    assert_eq!(compiled.functions, source_bodies, "{}", case.name);
    assert!(
        generic_bodies > 0,
        "{}: equality caller must retain its universal binder",
        case.name
    );
    assert_eq!(
        compiled
            .report
            .node_evaluations
            .iter()
            .filter(|(key, _)| key.kind == 110)
            .count(),
        eager_bodies
    );
    assert_eq!(compiled.control_flow.len(), compiled.analyses.len());
    assert!(compiled.analyses.iter().all(|analysis| analysis.passed()));
    for &kind in if check {
        &[][..]
    } else {
        &[102u32, 108, 109][..]
    } {
        let nodes: Vec<_> = compiled
            .report
            .node_evaluations
            .iter()
            .filter(|(key, _)| key.kind == kind)
            .collect();
        assert_eq!(
            nodes.len(),
            compiled.control_flow.len(),
            "{}:{kind}",
            case.name
        );
        assert!(nodes.iter().all(|(_, count)| *count == 1));
    }
    assert_eq!(
        compiled
            .report
            .node_evaluations
            .iter()
            .filter(|(key, _)| key.kind == 103)
            .count(),
        if check {
            0
        } else {
            compiled.control_flow.len()
        },
        "{}: object emission only in BUILD",
        case.name
    );
}
fn all_body_graph() {
    let mut failures = CellFailures::default();
    let mut modes = [[0; 2]; 4];
    let mut unused_failures = 0;
    for case in cases() {
        let (outcome, phase) = native(&case);
        // Attributed Runtime sources are executed by the CLI matrix; the
        // graph matrix covers their attribute-free twins.
        if case.source.starts_with("#![") && outcome == Outcome::Runtime {
            continue;
        }
        for workers in [1, 8] {
            for check in [true, false] {
                failures.check(
                    format!(
                        "native graph {} workers{workers} check{check} expected {outcome:?}/{phase:?}",
                        case.name
                    ),
                    || match compiler::compile_observed(case.source.to_owned(), workers, check) {
                        Ok(compiled) => {
                            assert_eq!(outcome, Outcome::Runtime, "{}", case.name);
                            successful_all_body_graph(&case, &compiled, check);
                        }
                        Err(failure) => {
                            assert_ne!(outcome, Outcome::Runtime, "{}:{failure}", case.name);
                            assert_native_diagnostic(&case, &failure.to_string());
                            let report = failure.report.expect("failure retains graph report");
                            assert!(report.capture_complete, "{}", case.name);
                            assert_eq!(report.events_dropped, 0);
                            assert!(report.failures > 0);
                            assert!(
                                !report.published_kind(105),
                                "{}: rejected source emitted object",
                                case.name
                            );
                            if matches!(phase, Phase::Attribute) {
                                assert!(
                                    report
                                        .node_evaluations
                                        .iter()
                                        .any(|(key, count)| key.kind == 107 && *count == 1)
                                );
                                assert!(
                                    !report
                                        .node_evaluations
                                        .iter()
                                        .any(|(key, _)| matches!(key.kind, 110..=112)),
                                    "{}: early syntax failure reached body graph",
                                    case.name
                                );
                            }
                            if case.name.ends_with("unused_bad_generic_body") {
                                let program = frontend::parse(case.source).unwrap();
                                let plan = frontend::PlanCtx::from_program(
                                    &program,
                                    &ir::PlanBudget::default(),
                                )
                                .unwrap();
                                let mut saw_unused = false;
                                for index in 0..plan.callable_count() {
                                    let declaration = plan.callable_decl(index).unwrap();
                                    if let frontend::CallableProvider::Source { .. } =
                                        plan.callable_provider(declaration).unwrap()
                                    {
                                        let body =
                                            plan.callable_body(&program, declaration).unwrap();
                                        if body.name.segments().last().is_some_and(|name| {
                                            program.names.spelling(*name).unwrap() == "bad"
                                        }) {
                                            assert!(
                                                plan.callable_env(declaration)
                                                    .unwrap()
                                                    .is_generic()
                                            );
                                            saw_unused = true;
                                        }
                                    }
                                }
                                assert!(
                                    saw_unused,
                                    "{}: genuinely unused universal bad body still rejected",
                                    case.name
                                );
                                unused_failures += 1;
                            }
                        }
                    },
                );
                modes[partition_index(outcome)][usize::from(!check)] += 1;
            }
        }
    }
    assert_eq!(modes, [[14, 14], [36, 36], [12, 12], [0, 0]]);
    failures.check("native graph unused universal rejection count", || {
        assert_eq!(unused_failures, 12)
    });
    failures.finish();
    eprintln!(
        "NATIVE PARAMETER EQUALITY ALL-BODY GRAPH:180 cells,modes{modes:?},4 unused universal bad-body rejection cells; all positive universal binders retained"
    );
}
#[test]
fn native_parameter_equality_all_body_graph_workers1_8_check_build() {
    const CHILD: &str = "JR_PARAMETER_EQUALITY_ALL_BODY_CHILD";
    if std::env::var_os(CHILD).is_some() {
        all_body_graph();
        return;
    }
    let scratch = Scratch::new();
    let mut command = Command::new(std::env::current_exe().unwrap());
    no_compiler_tools(&mut command, &scratch);
    command
        .args([
            "--exact",
            "native_parameter_equality_all_body_graph_workers1_8_check_build",
            "--nocapture",
        ])
        .env(CHILD, "1");
    // A failed full matrix can exceed one subprocess's 64 KiB output budget.
    let result = capture_bounded(&mut command, &scratch, 900, 8 * 1024 * 1024);
    // Forward every child cell receipt, including passes, to the root gate log.
    eprint!("{}", String::from_utf8_lossy(&result.stdout));
    eprint!("{}", String::from_utf8_lossy(&result.stderr));
    assert_eq!(
        result.code,
        0,
        "all-body graph child:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(
        String::from_utf8_lossy(&result.stderr)
            .contains("NATIVE PARAMETER EQUALITY ALL-BODY GRAPH:180 cells"),
        "missing complete graph receipt:{result:?}"
    );
}
