//! Frozen stock-first acceptance corpus for ASSOCIATED-EQUALITY.md and equality-contract-final.md.
//! Original JSON SHA256: 9de82105ccbfea8cccc8dcca460a84f1d7e93e8b5248482a02674d79efe5fa8f.
//! Append JSON SHA256: d237a12d78fd4cc7edf89855858061f217018fe35f6c91ae49f3719b9e919e36.
//! Additional JSON SHA256: 2d56af495eeb41632e9f4b9e7a72e2c86913d1f7e82f377c6fbbb9b4fe4d2bed.
//! Additional receipt: scratch/equality-tests-stock-20261017/results.json.
//! Each source batch was frozen before its native expectations. Original20 retain bytes/order.
//! Ordered source SHA256: 8b447cec8e57bcd0ff54b18d47734979c0dc169dee999c91dcf88be74f180b9a.
//! Original10 source SHA256: 5ab9ae4dc795d81ec491ed9c25197ecaaf1d1f950c968beca01d1347b8581528.
//! Append10 source SHA256: eec2fba54c2a2a25ce68c072b272c9b4ed068efd405082c099a2f2bd81ed87fb.
//! Supplemental zero-array JSON SHA256: c4c2fcc91795959bccdc0cd19bf8ed5373f630ac3cb98758c8972bb056e739c7.
//! Case 28 retains an independent CFG aggregate-comparison boundary. Case 46
//! is separately measured, executes the zero-array carrier without that operator.
//! Original exit calls are untouched: external-call resolution and projected where
//! syntax are earlier admission boundaries, not evidence about equality semantics.
//! Conflicting unused bounds are stock VALID, native Unsupported, never Invalid.
//! Native outcomes are acceptance requirements, NOT measured native results.
//! Run debug/release separately. No compiler-compatibility or speed claim.
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
    Body,
    ArrayComparison,
    ExternalExit,
    ParseProjection,
    Conflict,
    StaticLifetime,
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
}
fn cases() -> Vec<Case> {
    vec![
        // Source SHA256 e1a17d4cae9cee7027c6cb8d45714c7f02e7c98d4c3c935262fff18f1edf5e8b
        Case {
            name: "01_generic_forward",
            source: r#"trait Source { type Out; fn get() -> Self::Out; }
struct S;
impl Source for S { type Out = i32; fn get() -> i32 { 17 } }
fn forward<T: Source>() -> T::Out { T::get() }
fn main() { if forward::<S>() != 17 { std::process::exit(1); } println!("ok"); }
"#,
            source_bytes: 249,
            source_fingerprint: 16900025135850887326,
            stock_diagnostic: None,
            stdout: b"ok\n",
            outcome: Outcome::Unsupported,
            phase: Phase::ParseProjection,
        },
        // Source SHA256 87d52a49cf167d5f022ba7fae98043ff98e62b67190c3a99281c734fa65fdfed
        Case {
            name: "02_associated_equality",
            source: r#"trait Source { type Out; fn get() -> Self::Out; }
struct S;
impl Source for S { type Out = i32; fn get() -> i32 { 17 } }
fn take<T: Source<Out = i32>>() -> i32 { T::get() }
fn main() { if take::<S>() != 17 { std::process::exit(1); } println!("ok"); }
"#,
            source_bytes: 251,
            source_fingerprint: 1378467388530890083,
            stock_diagnostic: None,
            stdout: b"ok\n",
            outcome: Outcome::Unsupported,
            phase: Phase::ExternalExit,
        },
        // Source SHA256 6e12372e3fc2ce507c14e4c79483d335a96b06978f4dbc048bd7a536e089a074
        Case {
            name: "03_wrap_closed_equality",
            source: r#"trait Source { type Out; fn get(value: Self::Out) -> Self::Out; }
struct Wrap<U> { value: U }
impl<U> Source for Wrap<U> { type Out = U; fn get(value: U) -> U { value } }
fn close<T: Source<Out = i32>>(value: i32) -> i32 { T::get(value) }
fn main() { let w = Wrap { value: 23 }; if close::<Wrap<i32>>(w.value) != 23 { std::process::exit(1); } println!("ok"); }
"#,
            source_bytes: 361,
            source_fingerprint: 746155412715955163,
            stock_diagnostic: None,
            stdout: b"ok\n",
            outcome: Outcome::Unsupported,
            phase: Phase::ExternalExit,
        },
        // Source SHA256 4ea624781247b0ce927da4ab75d1b5230439c5794a2d5baca27a46122fc6fcfc
        Case {
            name: "04_noncopy_moved_once",
            source: r#"trait Source { type Out; fn get() -> Self::Out; }
struct Record { value: i32 }
struct S;
impl Source for S { type Out = Record; fn get() -> Record { Record { value: 29 } } }
fn take<T: Source<Out = Record>>() -> Record { let value = T::get(); value }
fn main() { let record = take::<S>(); if record.value != 29 { std::process::exit(1); } println!("ok"); }
"#,
            source_bytes: 356,
            source_fingerprint: 7453723518854153011,
            stock_diagnostic: None,
            stdout: b"ok\n",
            outcome: Outcome::Unsupported,
            phase: Phase::ExternalExit,
        },
        // Source SHA256 eeae904492030524323f1821482ae5f3f4f2520bc9beb1d42eaaadad36208ee2
        Case {
            name: "05_i128_tuple_equalities",
            source: r#"trait Source { type Out; fn get() -> Self::Out; }
struct Big; struct Pair;
impl Source for Big { type Out = i128; fn get() -> i128 { 170141183460469231731687303715884105727i128 } }
impl Source for Pair { type Out = (i32, bool); fn get() -> (i32, bool) { (31, true) } }
fn big<T: Source<Out = i128>>() -> i128 { T::get() }
fn pair<T: Source<Out = (i32, bool)>>() -> (i32, bool) { T::get() }
fn main() { if big::<Big>() != 170141183460469231731687303715884105727i128 { std::process::exit(1); } let p = pair::<Pair>(); if p.0 != 31 || !p.1 { std::process::exit(1); } println!("ok"); }
"#,
            source_bytes: 582,
            source_fingerprint: 6256024800572299810,
            stock_diagnostic: None,
            stdout: b"ok\n",
            outcome: Outcome::Unsupported,
            phase: Phase::ExternalExit,
        },
        // Source SHA256 81bc87985d694a5e98e355b79178f28d10f2dd56b3ed3ed8459552d3d8b40d7a
        Case {
            name: "06_wrong_equality_call",
            source: r#"trait Source { type Out; fn get() -> Self::Out; }
struct S;
impl Source for S { type Out = i32; fn get() -> i32 { 17 } }
fn wrong<T: Source<Out = bool>>() -> bool { T::get() }
fn main() { let _ = wrong::<S>(); }
"#,
            source_bytes: 212,
            source_fingerprint: 14942581886581612401,
            stock_diagnostic: Some("error[E0271]"),
            stdout: b"",
            outcome: Outcome::Invalid,
            phase: Phase::Body,
        },
        // Source SHA256 a29a170639183b0dc7a6474c1aa54b4aacc0d9c15ea2a022c3fb9e64ab1f82f4
        Case {
            name: "07_unused_bad_body",
            source: r#"trait Source { type Out; fn get() -> Self::Out; }
struct S;
impl Source for S { type Out = i32; fn get() -> i32 { 17 } }
fn unused<T: Source<Out = i32>>() -> bool { T::get() }
fn main() {}
"#,
            source_bytes: 189,
            source_fingerprint: 2450396930555974281,
            stock_diagnostic: Some("error[E0308]"),
            stdout: b"",
            outcome: Outcome::Invalid,
            phase: Phase::Body,
        },
        // Source SHA256 5e6557969ea4e16ec1f8d03c60cf338057ae2b164ad34d678b472408c8467f3a
        Case {
            name: "08_unresolved_generic_duplication",
            source: r#"trait Source { type Out; fn get() -> Self::Out; }
struct S;
impl Source for S { type Out = i32; fn get() -> i32 { 17 } }
fn duplicate<T: Source>(value: T::Out) -> (T::Out, T::Out) { (value, value) }
fn main() {}
"#,
            source_bytes: 212,
            source_fingerprint: 4224370094492200845,
            stock_diagnostic: Some("error[E0382]"),
            stdout: b"",
            outcome: Outcome::InvalidUnsupported,
            phase: Phase::ParseProjection,
        },
        // Source SHA256 9be7d43663f4a913da2b28c02120582b7728762dd3897d5098417baff96ef4eb
        Case {
            name: "09_copy_bound_and_equality_duplication",
            source: r#"trait Source { type Out; fn get() -> Self::Out; }
struct S;
impl Source for S { type Out = i32; fn get() -> i32 { 17 } }
fn bound<T: Source>(value: T::Out) -> (T::Out, T::Out) where T::Out: Copy { (value, value) }
fn equality<T: Source<Out = i32>>(value: T::Out) -> (T::Out, T::Out) { (value, value) }
fn main() { let a = bound::<S>(37); let b = equality::<S>(41); if a.0 != 37 || a.1 != 37 || b.0 != 41 || b.1 != 41 { std::process::exit(1); } println!("ok"); }
"#,
            source_bytes: 462,
            source_fingerprint: 16334128787973259919,
            stock_diagnostic: None,
            stdout: b"ok\n",
            outcome: Outcome::Unsupported,
            phase: Phase::ParseProjection,
        },
        // Source SHA256 1cae3dcdd478b2f2b237629f749fa2796b8045fb99a37b8e5450d4da9760dc40
        Case {
            name: "10_same_self_distinct_trait_arguments",
            source: r#"trait Source<A> { type Out; fn get() -> Self::Out; }
struct S;
impl Source<i32> for S { type Out = i128; fn get() -> i128 { 43 } }
impl Source<bool> for S { type Out = (i32, bool); fn get() -> (i32, bool) { (47, true) } }
fn first<T: Source<i32, Out = i128>>() -> i128 { <T as Source<i32>>::get() }
fn second<T: Source<bool, Out = (i32, bool)>>() -> (i32, bool) { <T as Source<bool>>::get() }
fn main() { let a = first::<S>(); let b = second::<S>(); if a != 43 || b.0 != 47 || !b.1 { std::process::exit(1); } println!("ok"); }
"#,
            source_bytes: 527,
            source_fingerprint: 17623759176448353613,
            stock_diagnostic: None,
            stdout: b"ok\n",
            outcome: Outcome::Unsupported,
            phase: Phase::ExternalExit,
        },
        // Source SHA256 8c77cf46b8118f0406c6274be55e136ceda1033b923e9e127c931fd2c7b08f2d
        Case {
            name: "11_generic_forward_positive_if",
            source: r#"trait Source { type Out; fn get() -> Self::Out; }
struct S;
impl Source for S { type Out = i32; fn get() -> i32 { 17 } }
fn forward<T: Source>() -> T::Out { T::get() }
fn main() { if !(forward::<S>() != 17) { println!("ok"); } }
"#,
            source_bytes: 229,
            source_fingerprint: 3801088847737584331,
            stock_diagnostic: None,
            stdout: b"ok\n",
            outcome: Outcome::Unsupported,
            phase: Phase::ParseProjection,
        },
        // Source SHA256 033f1ef08f0dec713207d48f93023c71d5b3467028a8963b635102c406cfbeb8
        Case {
            name: "12_associated_equality_positive_if",
            source: r#"trait Source { type Out; fn get() -> Self::Out; }
struct S;
impl Source for S { type Out = i32; fn get() -> i32 { 17 } }
fn take<T: Source<Out = i32>>() -> i32 { T::get() }
fn main() { if !(take::<S>() != 17) { println!("ok"); } }
"#,
            source_bytes: 231,
            source_fingerprint: 17390434835496201244,
            stock_diagnostic: None,
            stdout: b"ok\n",
            outcome: Outcome::Runtime,
            phase: Phase::None,
        },
        // Source SHA256 d12f35fde026457001b72a4158c15d049805a1f97a4665d665eba138cf086976
        Case {
            name: "13_wrap_closed_equality_positive_if",
            source: r#"trait Source { type Out; fn get(value: Self::Out) -> Self::Out; }
struct Wrap<U> { value: U }
impl<U> Source for Wrap<U> { type Out = U; fn get(value: U) -> U { value } }
fn close<T: Source<Out = i32>>(value: i32) -> i32 { T::get(value) }
fn main() { let w = Wrap { value: 23 }; if !(close::<Wrap<i32>>(w.value) != 23) { println!("ok"); } }
"#,
            source_bytes: 341,
            source_fingerprint: 5359157885941167780,
            stock_diagnostic: None,
            stdout: b"ok\n",
            outcome: Outcome::Runtime,
            phase: Phase::None,
        },
        // Source SHA256 a71177f85973392b9ab3bcf9c0f1805ca4c5383b6970f6a24c50a7e74812bdb6
        Case {
            name: "14_noncopy_moved_once_positive_if",
            source: r#"trait Source { type Out; fn get() -> Self::Out; }
struct Record { value: i32 }
struct S;
impl Source for S { type Out = Record; fn get() -> Record { Record { value: 29 } } }
fn take<T: Source<Out = Record>>() -> Record { let value = T::get(); value }
fn main() { let record = take::<S>(); if !(record.value != 29) { println!("ok"); } }
"#,
            source_bytes: 336,
            source_fingerprint: 16572022910407740086,
            stock_diagnostic: None,
            stdout: b"ok\n",
            outcome: Outcome::Runtime,
            phase: Phase::None,
        },
        // Source SHA256 9d26746d34a746e50fbe54cc87f424f98f7e8b9044e5a23e7d98e2908012d75f
        Case {
            name: "15_i128_tuple_equalities_positive_if",
            source: r#"trait Source { type Out; fn get() -> Self::Out; }
struct Big; struct Pair;
impl Source for Big { type Out = i128; fn get() -> i128 { 170141183460469231731687303715884105727i128 } }
impl Source for Pair { type Out = (i32, bool); fn get() -> (i32, bool) { (31, true) } }
fn big<T: Source<Out = i128>>() -> i128 { T::get() }
fn pair<T: Source<Out = (i32, bool)>>() -> (i32, bool) { T::get() }
fn main() { if !(big::<Big>() != 170141183460469231731687303715884105727i128) { let p = pair::<Pair>(); if !(p.0 != 31 || !p.1) { println!("ok"); } } }
"#,
            source_bytes: 542,
            source_fingerprint: 10755584806648513922,
            stock_diagnostic: None,
            stdout: b"ok\n",
            outcome: Outcome::Runtime,
            phase: Phase::None,
        },
        // Source SHA256 c1a379ddc6c85004006b6fa5f71650c82a7b5f7d5be2795c3f2d9fb88c7be861
        Case {
            name: "16_copy_bound_and_equality_duplication_positive_if",
            source: r#"trait Source { type Out; fn get() -> Self::Out; }
struct S;
impl Source for S { type Out = i32; fn get() -> i32 { 17 } }
fn bound<T: Source>(value: T::Out) -> (T::Out, T::Out) where T::Out: Copy { (value, value) }
fn equality<T: Source<Out = i32>>(value: T::Out) -> (T::Out, T::Out) { (value, value) }
fn main() { let a = bound::<S>(37); let b = equality::<S>(41); if !(a.0 != 37 || a.1 != 37 || b.0 != 41 || b.1 != 41) { println!("ok"); } }
"#,
            source_bytes: 442,
            source_fingerprint: 12682430148740272226,
            stock_diagnostic: None,
            stdout: b"ok\n",
            outcome: Outcome::Unsupported,
            phase: Phase::ParseProjection,
        },
        // Source SHA256 83e877d09fb1ed77d18dc23da4ee1d3e29211977658cc5c49d1cc92abcde03f6
        Case {
            name: "17_same_self_distinct_trait_arguments_positive_if",
            source: r#"trait Source<A> { type Out; fn get() -> Self::Out; }
struct S;
impl Source<i32> for S { type Out = i128; fn get() -> i128 { 43 } }
impl Source<bool> for S { type Out = (i32, bool); fn get() -> (i32, bool) { (47, true) } }
fn first<T: Source<i32, Out = i128>>() -> i128 { <T as Source<i32>>::get() }
fn second<T: Source<bool, Out = (i32, bool)>>() -> (i32, bool) { <T as Source<bool>>::get() }
fn main() { let a = first::<S>(); let b = second::<S>(); if !(a != 43 || b.0 != 47 || !b.1) { println!("ok"); } }
"#,
            source_bytes: 507,
            source_fingerprint: 15909509495321930124,
            stock_diagnostic: None,
            stdout: b"ok\n",
            outcome: Outcome::Runtime,
            phase: Phase::None,
        },
        // Source SHA256 080a1cda4b9a9af1f82946c0a4517db805b02024977b58beeab80e5fc248cc21
        Case {
            name: "18_duplicate_identical_equality_bounds",
            source: r#"trait Tr { type R; }
fn unused<T: Tr<R = i32> + Tr<R = i32>>() {}
fn main() {}
"#,
            source_bytes: 79,
            source_fingerprint: 17157517667301309856,
            stock_diagnostic: None,
            stdout: b"",
            outcome: Outcome::Runtime,
            phase: Phase::None,
        },
        // Source SHA256 b1c61a90440861e39e5c652fc92a3f71a614096f15ecad6eae95ac3c6c1261ab
        Case {
            name: "19_conflicting_equality_bounds_unused",
            source: r#"trait Tr { type R; }
fn unused<T: Tr<R = i32> + Tr<R = bool>>() {}
fn main() {}
"#,
            source_bytes: 80,
            source_fingerprint: 14117512763535996776,
            stock_diagnostic: None,
            stdout: b"",
            outcome: Outcome::Unsupported,
            phase: Phase::Conflict,
        },
        // Source SHA256 6ebc3500b952360b13ad2adfa46da75c40c18a56baf459942c63b5d83d6ab502
        Case {
            name: "20_duplicate_binding_single_trait",
            source: r#"trait Tr { type Out; }
fn unused<T: Tr<Out = i32, Out = i32>>() {}
fn main() {}
"#,
            source_bytes: 80,
            source_fingerprint: 5121237225845525617,
            stock_diagnostic: None,
            stdout: b"",
            outcome: Outcome::Runtime,
            phase: Phase::None,
        },
        // Source SHA256 435b3260f16be6c6c0e9f06e42c8fc18ec663ff95821553759b333d97ef37f28
        Case {
            name: "21_closed_scalar",
            source: r#"trait Source { type Out; fn get(self) -> Self::Out; }
struct S;
impl Source for S { type Out = i32; fn get(self) -> Self::Out { 53 } }
fn forward<T: Source<Out = i32>>(x: T) -> <T as Source>::Out { let y: <T as Source>::Out = x.get(); y }
fn main() { let x = forward(S); if x == 53 { println!("ok"); } }
"#,
            source_bytes: 304,
            source_fingerprint: 13785676347312303280,
            stock_diagnostic: None,
            stdout: b"ok\n",
            outcome: Outcome::Runtime,
            phase: Phase::None,
        },
        // Source SHA256 1bcc58d4837214c1b6b34fcc1861a1d5ac9de2303d4d2bbcda506728f36c5f84
        Case {
            name: "22_closed_tuple",
            source: r#"trait Source { type Out; fn get(self) -> Self::Out; }
struct S;
impl Source for S { type Out = (i32, bool); fn get(self) -> Self::Out { (59, true) } }
fn forward<T: Source<Out = (i32, bool)>>(x: T) -> <T as Source>::Out { let y: <T as Source>::Out = x.get(); y }
fn main() { let x = forward(S); if x.0 == 59 && x.1 { println!("ok"); } }
"#,
            source_bytes: 337,
            source_fingerprint: 2431905683653934228,
            stock_diagnostic: None,
            stdout: b"ok\n",
            outcome: Outcome::Runtime,
            phase: Phase::None,
        },
        // Source SHA256 69b27ddfc17e8db03366305a8b56e6480f55629ef44f2fef90fcc65bd0f4c6b9
        Case {
            name: "23_closed_wide",
            source: r#"trait Source { type Out; fn get(self) -> Self::Out; }
struct S;
impl Source for S { type Out = i128; fn get(self) -> Self::Out { 170141183460469231731687303715884105727i128 } }
fn forward<T: Source<Out = i128>>(x: T) -> <T as Source>::Out { let y: <T as Source>::Out = x.get(); y }
fn main() { let x = forward(S); if x == 170141183460469231731687303715884105727i128 { println!("ok"); } }
"#,
            source_bytes: 388,
            source_fingerprint: 4942735466729358632,
            stock_diagnostic: None,
            stdout: b"ok\n",
            outcome: Outcome::Runtime,
            phase: Phase::None,
        },
        // Source SHA256 221a7399f2633af417d2d5a60bf412ecf913d6c34374a1f35e830c486f14f0d6
        Case {
            name: "24_closed_u128",
            source: r#"trait Source { type Out; fn get(self) -> Self::Out; }
struct S;
impl Source for S { type Out = u128; fn get(self) -> Self::Out { 340282366920938463463374607431768211455u128 } }
fn forward<T: Source<Out = u128>>(x: T) -> <T as Source>::Out { let y: <T as Source>::Out = x.get(); y }
fn main() { let x = forward(S); if x == 340282366920938463463374607431768211455u128 { println!("ok"); } }
"#,
            source_bytes: 388,
            source_fingerprint: 4936099278223761894,
            stock_diagnostic: None,
            stdout: b"ok\n",
            outcome: Outcome::Runtime,
            phase: Phase::None,
        },
        // Source SHA256 189d27ab9860b86f03c4319ac87f0a318aa1efbc98ae60f60df7c708900c5816
        Case {
            name: "25_closed_unit",
            source: r#"trait Source { type Out; fn get(self) -> Self::Out; }
struct S;
impl Source for S { type Out = (); fn get(self) -> Self::Out { () } }
fn forward<T: Source<Out = ()>>(x: T) -> <T as Source>::Out { let y: <T as Source>::Out = x.get(); y }
fn main() { let x = forward(S); if x == () { println!("ok"); } }
"#,
            source_bytes: 302,
            source_fingerprint: 11755395714908234830,
            stock_diagnostic: None,
            stdout: b"ok\n",
            outcome: Outcome::Runtime,
            phase: Phase::None,
        },
        // Source SHA256 8627bf16fcb5c46ae624dcd5c90fb9d34283513308c7e062a0f6cba29f467667
        Case {
            name: "26_closed_float",
            source: r#"trait Source { type Out; fn get(self) -> Self::Out; }
struct S;
impl Source for S { type Out = f64; fn get(self) -> Self::Out { 1.5 } }
fn forward<T: Source<Out = f64>>(x: T) -> <T as Source>::Out { let y: <T as Source>::Out = x.get(); y }
fn main() { let x = forward(S); if x == 1.5 { println!("ok"); } }
"#,
            source_bytes: 306,
            source_fingerprint: 395405294743093786,
            stock_diagnostic: None,
            stdout: b"ok\n",
            outcome: Outcome::Runtime,
            phase: Phase::None,
        },
        // Source SHA256 4e1238cde3bade15af4ed4ab50624242228628535b10f70b196fbb13ccd8341e
        Case {
            name: "27_closed_array",
            source: r#"trait Source { type Out; fn get(self) -> Self::Out; }
struct S;
impl Source for S { type Out = [i32; 2]; fn get(self) -> Self::Out { [61, 67] } }
fn forward<T: Source<Out = [i32; 2]>>(x: T) -> <T as Source>::Out { let y: <T as Source>::Out = x.get(); y }
fn main() { let x = forward(S); if x[0] == 61 && x[1] == 67 { println!("ok"); } }
"#,
            source_bytes: 337,
            source_fingerprint: 12567804142681402853,
            stock_diagnostic: None,
            stdout: b"ok\n",
            outcome: Outcome::Runtime,
            phase: Phase::None,
        },
        // Source SHA256 b870e745f06afe9ccf52ee5a393c244ce2bd52963bdfb16b708cd7c5696fd0ef
        Case {
            name: "28_closed_zero_array",
            source: r#"trait Source { type Out; fn get(self) -> Self::Out; }
struct S;
impl Source for S { type Out = [i32; 0]; fn get(self) -> Self::Out { [] } }
fn forward<T: Source<Out = [i32; 0]>>(x: T) -> <T as Source>::Out { let y: <T as Source>::Out = x.get(); y }
fn main() { let x = forward(S); if x == [] { println!("ok"); } }
"#,
            source_bytes: 314,
            source_fingerprint: 17131078335978388802,
            stock_diagnostic: None,
            stdout: b"ok\n",
            outcome: Outcome::Unsupported,
            phase: Phase::ArrayComparison,
        },
        // Source SHA256 c5172e3e823b38121711f6321bc341069e3fec0349085598d5bb0ed220245727
        Case {
            name: "29_closed_noncopy",
            source: r#"struct Record { value: i32 }
trait Source { type Out; fn get(self) -> Self::Out; }
struct S;
impl Source for S { type Out = Record; fn get(self) -> Self::Out { Record { value: 71 } } }
fn forward<T: Source<Out = Record>>(x: T) -> Record { x.get() }
fn consume(x: Record) -> i32 { x.value }
fn main() { let x = forward(S); let y = x; if consume(y) == 71 { println!("ok"); } }
"#,
            source_bytes: 375,
            source_fingerprint: 375767292178127373,
            stock_diagnostic: None,
            stdout: b"ok\n",
            outcome: Outcome::Runtime,
            phase: Phase::None,
        },
        // Source SHA256 e48ff039d2baa5d37814554452c2733b4fd842eb2aa0c7a147e678bdb88a8a1a
        Case {
            name: "30_noncopy_reuse",
            source: r#"struct Record { value: i32 }
trait Source { type Out; fn get(self) -> Self::Out; }
struct S;
impl Source for S { type Out = Record; fn get(self) -> Self::Out { Record { value: 71 } } }
fn forward<T: Source<Out = Record>>(x: T) -> Record { x.get() }
fn consume(x: Record) -> i32 { x.value }
fn main() { let x = forward(S); let y = x; if consume(y) == consume(x) { println!("ok"); } }
"#,
            source_bytes: 383,
            source_fingerprint: 190739486082122706,
            stock_diagnostic: Some("error[E0382]"),
            stdout: b"",
            outcome: Outcome::Invalid,
            phase: Phase::Body,
        },
        // Source SHA256 77f7ab5340b955b80ce76f0bad3dad56226b731f3a2cd1b2072f00591088b78f
        Case {
            name: "31_wrong_equality_call",
            source: r#"trait Source { type Out; fn get(self) -> Self::Out; }
struct S;
impl Source for S { type Out = i32; fn get(self) -> Self::Out { 73 } }
fn forward<T: Source<Out = bool>>(x: T) -> bool { x.get() }
fn main() { let _ = forward(S); }
"#,
            source_bytes: 229,
            source_fingerprint: 2162584594843919281,
            stock_diagnostic: Some("error[E0271]"),
            stdout: b"",
            outcome: Outcome::Invalid,
            phase: Phase::Body,
        },
        // Source SHA256 64aae998cf93d11e5fc3cd27de96a0159c9e7379875e30d69e14e687c14ef4a9
        Case {
            name: "32_unused_bad_body",
            source: r#"trait Source { type Out; fn get(self) -> Self::Out; }
struct S;
fn unused<T: Source<Out = i32>>(x: T) -> bool { x.get() }
fn main() {}
"#,
            source_bytes: 135,
            source_fingerprint: 8564442103869298496,
            stock_diagnostic: Some("error[E0308]"),
            stdout: b"",
            outcome: Outcome::Invalid,
            phase: Phase::Body,
        },
        // Source SHA256 dc622fa3b870b26b50c941987f59a459cdf3b5d72d05bb12416932cc56774a83
        Case {
            name: "33_identical_bounds",
            source: r#"trait Source { type Out; fn get(self) -> Self::Out; }
struct S;
impl Source for S { type Out = i32; fn get(self) -> Self::Out { 79 } }
fn forward<T: Source<Out = i32> + Source<Out = i32>>(x: T) -> i32 { x.get() }
fn main() { if forward(S) == 79 { println!("ok"); } }
"#,
            source_bytes: 267,
            source_fingerprint: 6319986810116504559,
            stock_diagnostic: None,
            stdout: b"ok\n",
            outcome: Outcome::Runtime,
            phase: Phase::None,
        },
        // Source SHA256 c5866a27bb329645b81a1c461723c7629657460eaa72e028a87f4c47cedcb2ba
        Case {
            name: "34_identical_binding",
            source: r#"trait Source { type Out; fn get(self) -> Self::Out; }
struct S;
impl Source for S { type Out = i32; fn get(self) -> Self::Out { 79 } }
fn forward<T: Source<Out = i32, Out = i32>>(x: T) -> i32 { x.get() }
fn main() { if forward(S) == 79 { println!("ok"); } }
"#,
            source_bytes: 258,
            source_fingerprint: 12618090360884751333,
            stock_diagnostic: None,
            stdout: b"ok\n",
            outcome: Outcome::Runtime,
            phase: Phase::None,
        },
        // Source SHA256 5a1a19ea14e001fbf943fd8c21f1c8e5d181cdf1309d8ca09b5535250c217bd9
        Case {
            name: "35_conflicting_unused",
            source: r#"trait Source { type Out; fn get(self) -> Self::Out; }
struct S;
fn unused<T: Source<Out = i32> + Source<Out = bool>>(_x: T) {}
fn main() { println!("ok"); }
"#,
            source_bytes: 157,
            source_fingerprint: 10257981004212194990,
            stock_diagnostic: None,
            stdout: b"ok\n",
            outcome: Outcome::Unsupported,
            phase: Phase::Conflict,
        },
        // Source SHA256 61f9fb5bb311a649f2e7737076592b747dc7ae330e5012bed501e3f11d3939aa
        Case {
            name: "36_conflicting_unused_reversed",
            source: r#"trait Source { type Out; fn get(self) -> Self::Out; }
struct S;
fn unused<T: Source<Out = bool> + Source<Out = i32>>(_x: T) {}
fn main() { println!("ok"); }
"#,
            source_bytes: 157,
            source_fingerprint: 9527172851258065372,
            stock_diagnostic: None,
            stdout: b"ok\n",
            outcome: Outcome::Unsupported,
            phase: Phase::Conflict,
        },
        // Source SHA256 76fc73ffd704d98135cdc0e5fda7fcf590f8dfdcb9d39bf060803f87e36587c3
        Case {
            name: "37_parameter_rhs",
            source: r#"trait Source { type Out; fn get(self) -> Self::Out; }
struct S;
impl Source for S { type Out = i32; fn get(self) -> Self::Out { 83 } }
fn forward<T: Source<Out = U>, U>(x: T) -> U { x.get() }
fn main() { if forward::<S, i32>(S) == 83 { println!("ok"); } }
"#,
            source_bytes: 256,
            source_fingerprint: 13320683244852603914,
            stock_diagnostic: None,
            stdout: b"ok\n",
            outcome: Outcome::Runtime,
            phase: Phase::None,
        },
        // Source SHA256 08f7337bd6e0c0dc93a39a8b64de965d7907819c9899c63771c67cf4c0921f56
        Case {
            name: "38_no_equality",
            source: r#"trait Source { type Out; fn get(self) -> Self::Out; }
struct S;
impl Source for S { type Out = i32; fn get(self) -> Self::Out { 89 } }
fn forward<T: Source>(x: T) -> T::Out { x.get() }
fn main() { if forward(S) == 89 { println!("ok"); } }
"#,
            source_bytes: 239,
            source_fingerprint: 12216343063597757803,
            stock_diagnostic: None,
            stdout: b"ok\n",
            outcome: Outcome::Unsupported,
            phase: Phase::ParseProjection,
        },
        // Source SHA256 d98dfe72d19be0dfb6264dbb0a1834d214fc308c6a3a48c1cd93a230ae48a5d6
        Case {
            name: "39_out_copy",
            source: r#"trait Source { type Out: Copy; fn get(self) -> Self::Out; }
struct S;
impl Source for S { type Out = i32; fn get(self) -> Self::Out { 97 } }
fn forward<T: Source>(x: T) -> (T::Out, T::Out) { let y = x.get(); (y, y) }
fn main() { let x = forward(S); if x.0 == 97 && x.1 == 97 { println!("ok"); } }
"#,
            source_bytes: 297,
            source_fingerprint: 3635878469295405380,
            stock_diagnostic: None,
            stdout: b"ok\n",
            outcome: Outcome::Unsupported,
            phase: Phase::ParseProjection,
        },
        // Source SHA256 c38c80a7565f0d1d7e42a14c37bf4c7bc1b31ad085e04359130e9ea9976b1413
        Case {
            name: "40_where_equality",
            source: r#"trait Source { type Out; fn get(self) -> Self::Out; }
struct S;
impl Source for S { type Out = i32; fn get(self) -> Self::Out { 101 } }
fn forward<T>(x: T) -> i32 where T: Source<Out = i32> { x.get() }
fn main() { if forward(S) == 101 { println!("ok"); } }
"#,
            source_bytes: 257,
            source_fingerprint: 3653248674634559789,
            stock_diagnostic: None,
            stdout: b"ok\n",
            outcome: Outcome::Runtime,
            phase: Phase::None,
        },
        // Source SHA256 fd0155d67a147f526326546386084d4191d75df4f8fc06c2bace03e130fa800d
        Case {
            name: "41_forward_equality",
            source: r#"trait Source { type Out; fn get(self) -> Self::Out; }
struct S;
impl Source for S { type Out = i32; fn get(self) -> Self::Out { 103 } }
fn inner<T: Source<Out = i32>>(x: T) -> i32 { x.get() }
fn outer<T: Source<Out = i32>>(x: T) -> i32 { inner(x) }
fn main() { if outer(S) == 103 { println!("ok"); } }
"#,
            source_bytes: 302,
            source_fingerprint: 12210048169907301061,
            stock_diagnostic: None,
            stdout: b"ok\n",
            outcome: Outcome::Runtime,
            phase: Phase::None,
        },
        // Source SHA256 b4b04da49d15716b8a780862bdf4ccb41023e883f2d8e0cfbbb63bc05986b50d
        Case {
            name: "42_unused_equality_wrong_call",
            source: r#"trait Source { type Out; fn get(self) -> Self::Out; }
struct S;
impl Source for S { type Out = i32; fn get(self) -> Self::Out { 107 } }
fn unused<T: Source<Out = bool>>(_x: T) -> i32 { 109 }
fn main() { let _ = unused(S); }
"#,
            source_bytes: 224,
            source_fingerprint: 7484888943169963975,
            stock_diagnostic: Some("error[E0271]"),
            stdout: b"",
            outcome: Outcome::Invalid,
            phase: Phase::Body,
        },
        // Source SHA256 aab8c6d30388e5e0c6741920513f5799621c7d2f0c40f7828946eaf336f53927
        Case {
            name: "43_unused_valid_body",
            source: r#"trait Source { type Out; fn get(self) -> Self::Out; }
struct S;
fn unused<T: Source<Out = i32>>(x: T) -> i32 { x.get() }
fn main() { println!("ok"); }
"#,
            source_bytes: 151,
            source_fingerprint: 8552606275600668044,
            stock_diagnostic: None,
            stdout: b"ok\n",
            outcome: Outcome::Runtime,
            phase: Phase::None,
        },
        // Source SHA256 6b92c79498dfaa44635bbdebf3d41de77ab06296e0d105f54f34706c442b508e
        Case {
            name: "44_same_self_distinct_arguments",
            source: r#"trait Source<A> { type Out; fn get(self) -> Self::Out; }
struct S;
impl Source<i32> for S { type Out = i128; fn get(self) -> Self::Out { 113 } }
impl Source<bool> for S { type Out = bool; fn get(self) -> Self::Out { true } }
fn both<T: Source<i32, Out = i128> + Source<bool, Out = bool>>(x: T, y: T) -> (i128, bool) { (<T as Source<i32>>::get(x), <T as Source<bool>>::get(y)) }
fn main() { let x = both(S, S); if x.0 == 113 && x.1 { println!("ok"); } }
"#,
            source_bytes: 453,
            source_fingerprint: 6307367739369917769,
            stock_diagnostic: None,
            stdout: b"ok\n",
            outcome: Outcome::Runtime,
            phase: Phase::None,
        },
        // Source SHA256 545b83cfa2438ecd9e00e8e02d0d1d68bd6fd95869c3d875f2da300cc6a8e701
        Case {
            name: "45_reference_rhs",
            source: r#"trait Source { type Out; fn get(self) -> Self::Out; }
struct S;
fn unused<T: Source<Out = &'static i32>>(_x: T) {}
fn main() { println!("ok"); }
"#,
            source_bytes: 145,
            source_fingerprint: 11232338522751284758,
            stock_diagnostic: None,
            stdout: b"ok\n",
            outcome: Outcome::Unsupported,
            phase: Phase::StaticLifetime,
        },
        // Source SHA256 2b9667b49aa67cf0109c46b47732c3bb8b5656a5b312e51bc53b0f61bc430089
        Case {
            name: "46_zero_array_carrier",
            source: r#"trait Source { type Out; fn get(self) -> Self::Out; }
struct S;
impl Source for S { type Out = [i32; 0]; fn get(self) -> Self::Out { [] } }
fn forward<T: Source<Out = [i32; 0]>>(x: T) -> <T as Source>::Out { let y: <T as Source>::Out = x.get(); y }
fn consume(x: [i32; 0]) -> i32 { let _ = x; 127 }
fn main() { if consume(forward(S)) == 127 { println!("ok"); } }
"#,
            source_bytes: 363,
            source_fingerprint: 2319065687955526329,
            stock_diagnostic: None,
            stdout: b"ok\n",
            outcome: Outcome::Runtime,
            phase: Phase::None,
        },
    ]
}
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
fn associated_equality_frozen_stock_partition_and_source_bytes() {
    let cases = cases();
    assert_eq!(cases.len(), 46);
    let mut partition = [0; 4];
    let mut stock = [0; 2];
    for (index, case) in cases.iter().enumerate() {
        assert!(case.name.starts_with(&format!("{:02}_", index + 1)));
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
            matches!(case.outcome, Outcome::Invalid | Outcome::InvalidUnsupported),
            "{}",
            case.name
        );
        assert_eq!(
            case.phase == Phase::None,
            case.outcome == Outcome::Runtime,
            "{}",
            case.name
        );
    }
    assert_eq!(partition, [23, 6, 16, 1]);
    assert_eq!(stock, [39, 7]);
    assert_eq!(
        cases[..10]
            .iter()
            .filter(|c| c.stock_diagnostic.is_none())
            .count(),
        7
    );
    assert!(cases[10..20].iter().all(|c| c.stock_diagnostic.is_none()));
    for index in [0, 1, 2, 3, 4, 8, 9] {
        assert!(cases[index].source.contains("std::process::exit(1)"));
    }
    for index in [18, 34, 35] {
        assert_eq!(cases[index].phase, Phase::Conflict);
        assert!(cases[index].stock_diagnostic.is_none());
        assert_eq!(cases[index].outcome, Outcome::Unsupported);
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
            "jr-associated-equality-{}-{nonce}-{}",
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
    let expected = match case.phase {
        Phase::ExternalExit => "unresolved function",
        Phase::ArrayComparison => "aggregate operators and comparisons are unsupported",
        Phase::ParseProjection => {
            "unsupported: associated type projections are unsupported, found \"::\""
        }
        Phase::Conflict => "conflicting declared projection equalities",
        // This source is preserved, not rewritten to bypass its earlier lifetime parser boundary.
        Phase::StaticLifetime => "static and inferred lifetimes are unsupported",
        Phase::Body => {
            assert_eq!(case.outcome, Outcome::Invalid);
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
        "{} expected phase {:?}, snippet {expected:?}: {diagnostic}",
        case.name,
        case.phase
    );
    if case.name == "39_out_copy" {
        assert!(
            diagnostic.contains(&format!(":4:34: {expected}")),
            "{} expected recorded short-projection parser location:{diagnostic}",
            case.name
        );
    }
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
            eprintln!("ASSOCIATED EQUALITY CELL FAILURE: {failure}");
            self.0.push(failure);
        }
    }
    fn finish(self) {
        assert!(
            self.0.is_empty(),
            "{} associated equality failures:\n{}",
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
        "ASSOCIATED EQUALITY FROZEN BASELINE:{BASELINE_STOCK}\nACTUAL STOCK:\n{}",
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
                        "--edition=2024",
                        "--crate-name=associated_equality_oracle",
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
            if let Some(expected) = case.stock_diagnostic {
                assert_eq!(result.code, 1, "{}:{result:?}", case.name);
                assert!(
                    String::from_utf8_lossy(&result.stderr).contains(expected),
                    "{}:{result:?}",
                    case.name
                );
                assert!(!image.exists());
            } else {
                assert_eq!(result.code, 0, "{}:{result:?}", case.name);
                runtime(&image, &scratch, case.stdout);
            }
        });
    }
    assert_eq!(counts, [39, 7]);
    failures.finish();
    eprintln!(
        "STOCK ASSOCIATED EQUALITY:46 frozen sources,39 accepted with exact payload,7 rejected; original10+append10+additional25+supplemental1"
    );
}
#[test]
fn stock_associated_equality_oracle_edition2024_actual_toolchain() {
    stock_all();
}
#[test]
fn native_associated_equality_workers1_8_check_build_no_compiler_fallback() {
    // Aggregate a stock failure without preventing native coverage.
    let mut failures = CellFailures::default();
    failures.check("stock prerequisite", stock_all);
    let mut cells = 0;
    let mut modes = [[0; 2]; 4];
    let mut runtimes = 0;
    for case in cases() {
        for workers in [1, 8] {
            for check in [true, false] {
                failures.check(
                    format!(
                        "native CLI {} workers{workers} check{check} expected {:?}/{:?}",
                        case.name, case.outcome, case.phase
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
                            i32::from(case.outcome != Outcome::Runtime),
                            "{} workers{workers} check{check}:{result:?}",
                            case.name
                        );
                        if case.outcome != Outcome::Runtime {
                            assert!(result.stdout.is_empty(), "{}:{result:?}", case.name);
                            assert_native_diagnostic(
                                &case,
                                &String::from_utf8_lossy(&result.stderr),
                            );
                        }
                        if !check && case.outcome == Outcome::Runtime {
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
                modes[partition_index(case.outcome)][usize::from(!check)] += 1;
                cells += 1;
            }
        }
    }
    assert_eq!(cells, 184);
    assert_eq!(modes, [[46, 46], [12, 12], [32, 32], [2, 2]]);
    failures.check("native CLI successful runtime count", || {
        assert_eq!(runtimes, 46)
    });
    failures.finish();
    eprintln!(
        "NATIVE ASSOCIATED EQUALITY:184 CLI cells,modes{modes:?},46 native ELF payloads;23 Runtime,6 Invalid,16 stock-valid Unsupported,1 stock-invalid Unsupported"
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
        for workers in [1, 8] {
            for check in [true, false] {
                failures.check(
                    format!(
                        "native graph {} workers{workers} check{check} expected {:?}/{:?}",
                        case.name, case.outcome, case.phase
                    ),
                    || match compiler::compile_observed(case.source.to_owned(), workers, check) {
                        Ok(compiled) => {
                            assert_eq!(case.outcome, Outcome::Runtime, "{}", case.name);
                            successful_all_body_graph(&case, &compiled, check);
                        }
                        Err(failure) => {
                            assert_ne!(case.outcome, Outcome::Runtime, "{}:{failure}", case.name);
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
                            if matches!(case.phase, Phase::ParseProjection | Phase::StaticLifetime)
                            {
                                assert!(report.node_evaluations.iter().any(
                                    |(key, count)| key.kind == 121
                                        && key.params == [0]
                                        && *count == 1
                                ));
                                assert!(
                                    !report
                                        .node_evaluations
                                        .iter()
                                        .any(|(key, _)| matches!(key.kind, 110..=112)),
                                    "{}: early syntax failure reached body graph",
                                    case.name
                                );
                            }
                            if matches!(case.name, "07_unused_bad_body" | "32_unused_bad_body") {
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
                                            program.names.spelling(*name).unwrap() == "unused"
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
                modes[partition_index(case.outcome)][usize::from(!check)] += 1;
            }
        }
    }
    assert_eq!(modes, [[46, 46], [12, 12], [32, 32], [2, 2]]);
    failures.check("native graph unused universal rejection count", || {
        assert_eq!(unused_failures, 8)
    });
    failures.finish();
    eprintln!(
        "NATIVE ASSOCIATED EQUALITY ALL-BODY GRAPH:184 cells,modes{modes:?},8 unused universal bad-body rejection cells; all positive universal binders retained"
    );
}
#[test]
fn native_associated_equality_all_body_graph_workers1_8_check_build() {
    const CHILD: &str = "JR_ASSOCIATED_EQUALITY_ALL_BODY_CHILD";
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
            "native_associated_equality_all_body_graph_workers1_8_check_build",
            "--nocapture",
        ])
        .env(CHILD, "1");
    // A failed full matrix can exceed one subprocess's 64 KiB output budget.
    let result = capture_bounded(&mut command, &scratch, 240, 8 * 1024 * 1024);
    assert_eq!(
        result.code,
        0,
        "all-body graph child:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(
        String::from_utf8_lossy(&result.stderr)
            .contains("NATIVE ASSOCIATED EQUALITY ALL-BODY GRAPH:184 cells"),
        "missing complete graph receipt:{result:?}"
    );
}
