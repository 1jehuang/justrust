//! Associated declaration bounds: frozen stock sources and observed native phases.
//! Contract: ASSOCIATED-BOUNDS.md. No full Rust or general projection support claim.
//! Original 29 exact sources across cases(18), controls(8), supplemental(3) are unchanged.
//! Original receipt: associated-bounds-stock/results.json
//! SHA256: 9b4a51ddbd5a7773e6d1f5992bb33083f1bd90ebb40c6362855542fb6a094355
//! Append receipt: associated-bounds-stock-append/receipt.json
//! SHA256: e14960c7ce019ef53084fd257803f9627cbba99f623d2d7544b5bfb87d92b268
//! Native phases frozen from the full graph/CLI workers1/8 CHECK/BUILD output in
//! 942230mfnq.output. Original29: Runtime2, Invalid10, Unsupported17.
//! Append11: Runtime6, Invalid4, Unsupported1 (nominal-self-pattern boundary).
//! Total40: Runtime8, Invalid14, Unsupported18. Earlier syntax, resolution and
//! inference failures do not demonstrate associated-bound proof coverage.
//! The frozen user trait named Copy hits its scalar impl self-pattern boundary,
//! not associated user-trait validation. No source was rewritten to bypass it.
//! These assertions still require the final debug/release acceptance gates.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
use justrust_engine::compiler::{self, frontend, ir};
use justrust_engine::{Key, Report};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    Runtime,
    Invalid,
    Unsupported,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Boundary {
    None,
    MoveAnalysis,
    TraitObligation,
    TypeResolution,
    Syntax(&'static str),
    FunctionResolution,
    GenericInference,
    AssociatedEquality,
    NominalSelfPattern,
}
struct Case {
    name: &'static str,
    group: &'static str,
    source: &'static str,
    source_bytes: usize,
    source_fingerprint: u64,
    stock_errors: &'static [&'static str],
    outcome: Outcome,
    boundary: Boundary,
}
fn cases() -> Vec<Case> {
    vec![
        // Source SHA256 6432bf6ba2ec00a7d8d452634afa690c3aa3a5fa377140e47d93a84ce92cbb11
        Case {
            name: "cases_01_bound_implies_generic_copy",
            group: "cases",
            source: r###"trait Source { type Out: Copy; fn get(self) -> Self::Out; }
fn dup<T: Source<Out = U>, U>(x: T) -> (U, U) { let y = x.get(); (y, y) }
struct Wrap<T>(T);
impl<T: Copy> Source for Wrap<T> { type Out = T; fn get(self) -> T { self.0 } }
fn main() { let _ = dup(Wrap(7u8)); println!("ok"); }
"###,
            source_bytes: 287,
            source_fingerprint: 3275860743914007048,
            stock_errors: &["E0382"],
            outcome: Outcome::Invalid,
            boundary: Boundary::MoveAnalysis,
        },
        // Source SHA256 36affe439b686c8889cdd48a2f22499567735a9aa40dff84778b8e037a1000c3
        Case {
            name: "cases_02_generic_impl_with_copy",
            group: "cases",
            source: r###"trait Source { type Out: Copy; fn get(self) -> Self::Out; }
struct Wrap<T>(T);
impl<T: Copy> Source for Wrap<T> { type Out = T; fn get(self) -> T { self.0 } }
fn main() { let _ = Wrap(1u32).get(); println!("ok"); }
"###,
            source_bytes: 215,
            source_fingerprint: 2505224410299946570,
            stock_errors: &[],
            outcome: Outcome::Runtime,
            boundary: Boundary::None,
        },
        // Source SHA256 0152468913eb8cdb22bdb3bba493a0bdf1b7cb59c2590937def6bad9116df23d
        Case {
            name: "cases_03_generic_impl_without_copy",
            group: "cases",
            source: r###"trait Source { type Out: Copy; fn get(self) -> Self::Out; }
struct Wrap<T>(T);
impl<T> Source for Wrap<T> { type Out = T; fn get(self) -> T { self.0 } }
fn main() { println!("ok"); }
"###,
            source_bytes: 183,
            source_fingerprint: 5685655150167494844,
            stock_errors: &["E0277"],
            outcome: Outcome::Invalid,
            boundary: Boundary::TraitObligation,
        },
        // Source SHA256 f93bf4715bcf0310189d451fc3d1424faecbfce49153d09363125ded8addf0db
        Case {
            name: "cases_04_noncopy_rhs_unused",
            group: "cases",
            source: r###"trait Source { type Out: Copy; fn get(self) -> Self::Out; }
struct Bad;
impl Source for Bad { type Out = String; fn get(self) -> String { String::new() } }
fn main() { println!("ok"); }
"###,
            source_bytes: 186,
            source_fingerprint: 9902833918253631507,
            stock_errors: &["E0277"],
            outcome: Outcome::Unsupported,
            boundary: Boundary::TypeResolution,
        },
        // Source SHA256 19d3693c892444cfae08593ece03eb13255fb5811ec2077a0f6a545646ab2094
        Case {
            name: "cases_05_tuple_copy",
            group: "cases",
            source: r###"trait Source { type Out: Copy; fn get(self) -> Self::Out; }
fn dup<T: Source<Out = U>, U>(x: T) -> (U, U) { let y = x.get(); (y, y) }
struct Wrap<T>(T);
impl<T: Copy> Source for Wrap<T> { type Out = T; fn get(self) -> T { self.0 } }
fn main() { let _ = dup(Wrap((1u8, 2u64, true))); println!("ok"); }
"###,
            source_bytes: 301,
            source_fingerprint: 1697987148083301104,
            stock_errors: &["E0382"],
            outcome: Outcome::Invalid,
            boundary: Boundary::MoveAnalysis,
        },
        // Source SHA256 b9b94c5f7ab4c7fc9dda197d25a82c61df63d58eaa5154e447750c018d3b5dc8
        Case {
            name: "cases_06_wide_copy",
            group: "cases",
            source: r###"trait Source { type Out: Copy; fn get(self) -> Self::Out; }
fn dup<T: Source<Out = U>, U>(x: T) -> (U, U) { let y = x.get(); (y, y) }
struct Wrap<T>(T);
impl<T: Copy> Source for Wrap<T> { type Out = T; fn get(self) -> T { self.0 } }
fn main() { let _ = dup(Wrap(([3u64; 64], 4u128))); println!("ok"); }
"###,
            source_bytes: 303,
            source_fingerprint: 5949057885420266546,
            stock_errors: &["E0382"],
            outcome: Outcome::Invalid,
            boundary: Boundary::MoveAnalysis,
        },
        // Source SHA256 f65babe522c17e8fca8e910fb803c043da454e2c54237de01c632e3a39c3c272
        Case {
            name: "cases_07_equality_mismatch",
            group: "cases",
            source: r###"trait Source { type Out: Copy; fn get(self) -> Self::Out; }
fn dup<T: Source<Out = U>, U>(x: T) -> (U, U) { let y = x.get(); (y, y) }
struct Wrap<T>(T);
impl<T: Copy> Source for Wrap<T> { type Out = T; fn get(self) -> T { self.0 } }
fn main() { let _ = dup::<Wrap<u8>, u16>(Wrap(1u8)); println!("ok"); }
"###,
            source_bytes: 304,
            source_fingerprint: 7466752687292535547,
            stock_errors: &["E0271", "E0382"],
            outcome: Outcome::Invalid,
            boundary: Boundary::MoveAnalysis,
        },
        // Source SHA256 6d92a1a7e385542992afa1a51588293a499568a6fdd64f23f52a68a2c0adad50
        Case {
            name: "cases_08_wrong_other_trait_out",
            group: "cases",
            source: r###"trait Source { type Out; fn get(self) -> Self::Out; }
trait Other { type Out: Copy; }
fn dup<T: Source<Out = U> + Other, U>(x: T) -> (U, U) { let y = x.get(); (y, y) }
fn main() { println!("ok"); }
"###,
            source_bytes: 198,
            source_fingerprint: 4421128326447970525,
            stock_errors: &["E0382"],
            outcome: Outcome::Invalid,
            boundary: Boundary::MoveAnalysis,
        },
        // Source SHA256 a25eedefaf97da01d7e9cbb959876428cd77834d096db58f1ef6c2d21e3e1454
        Case {
            name: "cases_09_no_associated_bound",
            group: "cases",
            source: r###"trait Source { type Out; fn get(self) -> Self::Out; }
fn dup<T: Source<Out = U>, U>(x: T) -> (U, U) { let y = x.get(); (y, y) }
fn main() { println!("ok"); }
"###,
            source_bytes: 158,
            source_fingerprint: 9028173797576500954,
            stock_errors: &["E0382"],
            outcome: Outcome::Invalid,
            boundary: Boundary::MoveAnalysis,
        },
        // Source SHA256 ed39df97f8c9c55ea4c7d9a791310c8b16686706787d30d26c8f80f15a7b68c5
        Case {
            name: "cases_10_trait_args_distinct_outputs",
            group: "cases",
            source: r###"struct A; struct B; struct Both;
trait Source<K> { type Out: Copy; fn get(self) -> Self::Out; }
impl Source<A> for Both { type Out = u8; fn get(self) -> u8 { 1 } }
impl Source<B> for Both { type Out = u16; fn get(self) -> u16 { 2 } }
fn dup<K, T: Source<K, Out = U>, U>(x: T) -> (U, U) { let y = x.get(); (y, y) }
fn main() { let _: (u8, u8) = dup::<A, Both, u8>(Both); let _: (u16, u16) = dup::<B, Both, u16>(Both); println!("ok"); }
"###,
            source_bytes: 435,
            source_fingerprint: 1779394149108083572,
            stock_errors: &["E0382"],
            outcome: Outcome::Invalid,
            boundary: Boundary::MoveAnalysis,
        },
        // Source SHA256 0b75644f1d7ccfb657cf3ec0bf11b72a4715b8e600cb260df0c31b8fcd2255db
        Case {
            name: "cases_11_wrong_trait_argument_bound",
            group: "cases",
            source: r###"struct A; struct B;
trait Source<K> { type Out; fn get(self) -> Self::Out; }
fn dup<T, U>(x: T) -> (U, U) where T: Source<A> + Source<B, Out = U>, <T as Source<A>>::Out: Copy { let y = <T as Source<B>>::get(x); (y, y) }
fn main() { println!("ok"); }
"###,
            source_bytes: 250,
            source_fingerprint: 154388974037743026,
            stock_errors: &["E0382"],
            outcome: Outcome::Unsupported,
            boundary: Boundary::Syntax(":3:71: expected ASCII identifier, found \"<\""),
        },
        // Source SHA256 ebd39024300439f2a385fee8f7618f28aadd53ff0571ab780664e622785044ae
        Case {
            name: "cases_12_method_local_u_unrelated",
            group: "cases",
            source: r###"trait Source { type Out: Copy; fn get(self) -> Self::Out; }
trait Local: Source { fn dup_local<U>(&self, y: U) -> (U, U) { (y, y) } }
fn main() { println!("ok"); }
"###,
            source_bytes: 164,
            source_fingerprint: 14887849515335975279,
            stock_errors: &["E0382"],
            outcome: Outcome::Unsupported,
            boundary: Boundary::Syntax(
                ":2:62: unsupported: generic trait methods are unsupported, found \"{\"",
            ),
        },
        // Source SHA256 1c288953a7d5f3fbe889bf65bd215bc40873d2871f6e3c621dac54981dd24342
        Case {
            name: "cases_13_method_local_u_equal_projection",
            group: "cases",
            source: r###"trait Source { type Out: Copy; fn get(self) -> Self::Out; }
trait Local: Source { fn dup_local<U>(&self, y: U) -> (U, U) where Self: Source<Out = U> { (y, y) } }
struct Wrap<T>(T);
impl<T: Copy> Source for Wrap<T> { type Out = T; fn get(self) -> T { self.0 } }
impl<T: Copy> Local for Wrap<T> {}
fn main() { let _ = Wrap(1u8).dup_local(2u8); println!("ok"); }
"###,
            source_bytes: 360,
            source_fingerprint: 17832477138142408092,
            stock_errors: &["E0382"],
            outcome: Outcome::Unsupported,
            boundary: Boundary::Syntax(
                ":2:68: unsupported: Self where predicates are unsupported, found \"Self\"",
            ),
        },
        // Source SHA256 574d84d369066c2019e16de4c4516873058c9cef4653e9ca03e81d64dd10b031
        Case {
            name: "cases_14_implicit_sized_noncopy",
            group: "cases",
            source: r###"trait Source { type Out; fn get(self) -> Self::Out; }
struct Item;
impl Source for Item { type Out = String; fn get(self) -> String { String::new() } }
fn main() { let _ = Item.get(); println!("ok"); }
"###,
            source_bytes: 202,
            source_fingerprint: 16935682751661139034,
            stock_errors: &[],
            outcome: Outcome::Unsupported,
            boundary: Boundary::TypeResolution,
        },
        // Source SHA256 182d3f3cfebd530311def92881fc0a4a9dd0609166739f283e85a7d179f9376d
        Case {
            name: "cases_15_explicit_sized_noncopy",
            group: "cases",
            source: r###"trait Source { type Out: Sized; fn get(self) -> Self::Out; }
struct Item;
impl Source for Item { type Out = String; fn get(self) -> String { String::new() } }
fn main() { let _ = Item.get(); println!("ok"); }
"###,
            source_bytes: 209,
            source_fingerprint: 15008595178173171619,
            stock_errors: &[],
            outcome: Outcome::Unsupported,
            boundary: Boundary::TypeResolution,
        },
        // Source SHA256 02ce2875facb2879ddb84c1caf6d11576c0ef3d50247fb12c9f2eb6c83d8df53
        Case {
            name: "cases_16_explicit_sized_not_copy",
            group: "cases",
            source: r###"trait Source { type Out: Sized; fn get(self) -> Self::Out; }
fn dup<T: Source<Out = U>, U>(x: T) -> (U, U) { let y = x.get(); (y, y) }
fn main() { println!("ok"); }
"###,
            source_bytes: 165,
            source_fingerprint: 3478679577213381991,
            stock_errors: &["E0382"],
            outcome: Outcome::Invalid,
            boundary: Boundary::MoveAnalysis,
        },
        // Source SHA256 4ef9767323dc7b0ffc275e9de930297e44c2096518bfbd40500f13e3ae92e935
        Case {
            name: "cases_17_projection_where_copy_baseline",
            group: "cases",
            source: r###"trait Source where Self::Out: Copy { type Out; fn get(self) -> Self::Out; }
fn dup<T: Source<Out = U>, U>(x: T) -> (U, U) { let y = x.get(); (y, y) }
struct Wrap<T>(T);
impl<T: Copy> Source for Wrap<T> { type Out = T; fn get(self) -> T { self.0 } }
fn main() { let _ = dup(Wrap(1u8)); println!("ok"); }
"###,
            source_bytes: 303,
            source_fingerprint: 17152649042630584479,
            stock_errors: &["E0382"],
            outcome: Outcome::Unsupported,
            boundary: Boundary::Syntax(
                ":1:20: unsupported: Self where predicates are unsupported, found \"Self\"",
            ),
        },
        // Source SHA256 0d49ef2900aef6804d543489efd8dfd5227c255ffaa3a50a91597f75df3d398f
        Case {
            name: "cases_18_shared_reference_noncopy_referent",
            group: "cases",
            source: r###"trait Source { type Out: Copy; fn get(self) -> Self::Out; }
fn dup<T: Source<Out = U>, U>(x: T) -> (U, U) { let y = x.get(); (y, y) }
struct Wrap<T>(T);
impl<T: Copy> Source for Wrap<T> { type Out = T; fn get(self) -> T { self.0 } }
fn main() { let text = String::new(); let _ = dup(Wrap(&text)); println!("ok"); }
"###,
            source_bytes: 315,
            source_fingerprint: 16186225046686624671,
            stock_errors: &["E0382"],
            outcome: Outcome::Unsupported,
            boundary: Boundary::FunctionResolution,
        },
        // Source SHA256 f06ce6ead5e4898114edf2fd1ddf8d3fc3466fc495902d8d66c2b2a5d1634884
        Case {
            name: "controls_01_bound_implies_generic_copy_direct_projection_no_U",
            group: "controls",
            source: r###"trait Source { type Out: Copy; fn get(self) -> Self::Out; }
fn dup<T: Source>(x: T) -> (T::Out, T::Out) { let y = x.get(); (y, y) }
struct Wrap<T>(T);
impl<T: Copy> Source for Wrap<T> { type Out = T; fn get(self) -> T { self.0 } }
fn main() { let _ = dup(Wrap(7u8)); println!("ok"); }
"###,
            source_bytes: 285,
            source_fingerprint: 4960240954725364229,
            stock_errors: &[],
            outcome: Outcome::Unsupported,
            boundary: Boundary::Syntax(
                ":2:30: unsupported: associated type projections are unsupported, found \"::\"",
            ),
        },
        // Source SHA256 7aa8f32112abcd93be7ec393c8b3e578bdaff4b2329a5b1a1f7991609ba508e7
        Case {
            name: "controls_05_tuple_copy_explicit_U_Copy",
            group: "controls",
            source: r###"trait Source { type Out: Copy; fn get(self) -> Self::Out; }
fn dup<T: Source<Out = U>, U: Copy>(x: T) -> (U, U) { let y = x.get(); (y, y) }
struct Wrap<T>(T);
impl<T: Copy> Source for Wrap<T> { type Out = T; fn get(self) -> T { self.0 } }
fn main() { let _ = dup(Wrap((1u8, 2u64, true))); println!("ok"); }
"###,
            source_bytes: 307,
            source_fingerprint: 16339857724296205881,
            stock_errors: &[],
            outcome: Outcome::Unsupported,
            boundary: Boundary::GenericInference,
        },
        // Source SHA256 0fba7ab63aa1c2afbda85f4d45e9026192aefe4ac124720f0ea1db6eb34b37e6
        Case {
            name: "controls_06_wide_copy_explicit_U_Copy",
            group: "controls",
            source: r###"trait Source { type Out: Copy; fn get(self) -> Self::Out; }
fn dup<T: Source<Out = U>, U: Copy>(x: T) -> (U, U) { let y = x.get(); (y, y) }
struct Wrap<T>(T);
impl<T: Copy> Source for Wrap<T> { type Out = T; fn get(self) -> T { self.0 } }
fn main() { let _ = dup(Wrap(([3u64; 64], 4u128))); println!("ok"); }
"###,
            source_bytes: 309,
            source_fingerprint: 9776551712501030359,
            stock_errors: &[],
            outcome: Outcome::Unsupported,
            boundary: Boundary::GenericInference,
        },
        // Source SHA256 3f9f331d7c37589f4035b69b0fb8e7c076be41ec9be53f25e5ed47ec99811362
        Case {
            name: "controls_07_equality_mismatch_explicit_U_Copy",
            group: "controls",
            source: r###"trait Source { type Out: Copy; fn get(self) -> Self::Out; }
fn dup<T: Source<Out = U>, U: Copy>(x: T) -> (U, U) { let y = x.get(); (y, y) }
struct Wrap<T>(T);
impl<T: Copy> Source for Wrap<T> { type Out = T; fn get(self) -> T { self.0 } }
fn main() { let _ = dup::<Wrap<u8>, u16>(Wrap(1u8)); println!("ok"); }
"###,
            source_bytes: 310,
            source_fingerprint: 657425199363921488,
            stock_errors: &["E0271"],
            outcome: Outcome::Invalid,
            boundary: Boundary::AssociatedEquality,
        },
        // Source SHA256 c9dc5f8818126319f01117db46c41c373eede04042a4f302f155eb3e4afe68a8
        Case {
            name: "controls_10_trait_args_distinct_outputs_explicit_U_Copy",
            group: "controls",
            source: r###"struct A; struct B; struct Both;
trait Source<K> { type Out: Copy; fn get(self) -> Self::Out; }
impl Source<A> for Both { type Out = u8; fn get(self) -> u8 { 1 } }
impl Source<B> for Both { type Out = u16; fn get(self) -> u16 { 2 } }
fn dup<K, T: Source<K, Out = U>, U: Copy>(x: T) -> (U, U) { let y = x.get(); (y, y) }
fn main() { let _: (u8, u8) = dup::<A, Both, u8>(Both); let _: (u16, u16) = dup::<B, Both, u16>(Both); println!("ok"); }
"###,
            source_bytes: 441,
            source_fingerprint: 3245472380748049557,
            stock_errors: &[],
            outcome: Outcome::Runtime,
            boundary: Boundary::None,
        },
        // Source SHA256 24ec9653bc068e99be7191e15035dfeeab06c02204f23f1a854cd1c88be698a7
        Case {
            name: "controls_13_method_local_u_equal_projection_explicit_method_U_Copy",
            group: "controls",
            source: r###"trait Source { type Out: Copy; fn get(self) -> Self::Out; }
trait Local: Source { fn dup_local<U: Copy>(&self, y: U) -> (U, U) where Self: Source<Out = U> { (y, y) } }
struct Wrap<T>(T);
impl<T: Copy> Source for Wrap<T> { type Out = T; fn get(self) -> T { self.0 } }
impl<T: Copy> Local for Wrap<T> {}
fn main() { let _ = Wrap(1u8).dup_local(2u8); println!("ok"); }
"###,
            source_bytes: 366,
            source_fingerprint: 18326632952869743979,
            stock_errors: &[],
            outcome: Outcome::Unsupported,
            boundary: Boundary::Syntax(
                ":2:74: unsupported: Self where predicates are unsupported, found \"Self\"",
            ),
        },
        // Source SHA256 09269743abc5ddf491f7e464aac7617261af42b7e9cfa17e837081d1457f29ce
        Case {
            name: "controls_17_projection_where_copy_baseline_direct_projection_no_U",
            group: "controls",
            source: r###"trait Source where Self::Out: Copy { type Out; fn get(self) -> Self::Out; }
fn dup<T: Source>(x: T) -> (T::Out, T::Out) { let y = x.get(); (y, y) }
struct Wrap<T>(T);
impl<T: Copy> Source for Wrap<T> { type Out = T; fn get(self) -> T { self.0 } }
fn main() { let _ = dup(Wrap(1u8)); println!("ok"); }
"###,
            source_bytes: 301,
            source_fingerprint: 16677748605990770074,
            stock_errors: &[],
            outcome: Outcome::Unsupported,
            boundary: Boundary::Syntax(
                ":1:20: unsupported: Self where predicates are unsupported, found \"Self\"",
            ),
        },
        // Source SHA256 b9b2ff6352f7da7d9b00e9ac40be0f9ca6cbf409ab1a7d970a075c033df1560c
        Case {
            name: "controls_18_shared_reference_noncopy_referent_explicit_U_Copy",
            group: "controls",
            source: r###"trait Source { type Out: Copy; fn get(self) -> Self::Out; }
fn dup<T: Source<Out = U>, U: Copy>(x: T) -> (U, U) { let y = x.get(); (y, y) }
struct Wrap<T>(T);
impl<T: Copy> Source for Wrap<T> { type Out = T; fn get(self) -> T { self.0 } }
fn main() { let text = String::new(); let _ = dup(Wrap(&text)); println!("ok"); }
"###,
            source_bytes: 321,
            source_fingerprint: 13364298836486185786,
            stock_errors: &[],
            outcome: Outcome::Unsupported,
            boundary: Boundary::FunctionResolution,
        },
        // Source SHA256 0ff3b68df5d7adc75e3844bcabe4e91d6ded9cd2995c356f48053c94a9fe5302
        Case {
            name: "supplemental_19_unused_contradictory_owned_dup",
            group: "supplemental",
            source: r###"trait Source { type Out: Copy; fn get(self) -> Self::Out; }
fn impossible<T: Source<Out = String>>(x: T) -> (String, String) { let y = x.get(); (y, y) }
fn main() { println!("ok"); }
"###,
            source_bytes: 183,
            source_fingerprint: 2360135437069054732,
            stock_errors: &["E0382"],
            outcome: Outcome::Unsupported,
            boundary: Boundary::TypeResolution,
        },
        // Source SHA256 af5323215d4baf8edef0025723c5167ffc4f56e84a7505277fa7421236cc34c3
        Case {
            name: "supplemental_20_user_trait_named_copy",
            group: "supplemental",
            source: r###"trait Copy {}
impl Copy for String {}
trait Source { type Out: Copy; fn get(self) -> Self::Out; }
fn dup<T: Source<Out = String>>(x: T) -> (String, String) { let y = x.get(); (y, y) }
fn main() { println!("ok"); }
"###,
            source_bytes: 214,
            source_fingerprint: 15151064166079926370,
            stock_errors: &["E0382"],
            outcome: Outcome::Unsupported,
            boundary: Boundary::TypeResolution,
        },
        // Source SHA256 fef4cc1a3de789f45d3cd9d5503dc475a91bdbc877280c057e08a3773a4952fd
        Case {
            name: "supplemental_19_control_contradictory_without_dup",
            group: "supplemental",
            source: r###"trait Source { type Out: Copy; fn get(self) -> Self::Out; }
fn impossible<T: Source<Out = String>>(x: T) -> String { x.get() }
fn main() { println!("ok"); }
"###,
            source_bytes: 157,
            source_fingerprint: 14345291270731113107,
            stock_errors: &[],
            outcome: Outcome::Unsupported,
            boundary: Boundary::TypeResolution,
        },
        // Source SHA256 6e4f6b0b00b40477eceb231ca2e618df34dfe9ba504a7838263be6937f232014
        Case {
            name: "append_scalar_explicit_copy",
            group: "append",
            source: r###"trait Source: Sized { type Out: Copy; fn get(self) -> <Self as Source>::Out; }
struct Wrap<T>(T);
impl<T: Copy> Source for Wrap<T> { type Out = T; fn get(self) -> <Self as Source>::Out { self.0 } }
fn dup<T: Source<Out = U>, U: Copy>(x: T) -> (U,U) { let y = <T as Source>::get(x); (y,y) }
fn main() { let p=dup::<Wrap<i32>,i32>(Wrap(41)); if p.0==41 { if p.1==41 { println!("ok"); } } }
"###,
            source_bytes: 388,
            source_fingerprint: 3668011737800100676,
            stock_errors: &[],
            outcome: Outcome::Runtime,
            boundary: Boundary::None,
        },
        // Source SHA256 b694e0b92f4d2f4678963536478df14b1a78cd2a36c5919ae59639b5120b5c81
        Case {
            name: "append_tuple_full_payload",
            group: "append",
            source: r###"trait Source: Sized { type Out: Copy; fn get(self) -> <Self as Source>::Out; }
struct Wrap<T>(T);
impl<T: Copy> Source for Wrap<T> { type Out = T; fn get(self) -> <Self as Source>::Out { self.0 } }
fn dup<T: Source<Out = U>, U: Copy>(x: T) -> (U,U) { let y = <T as Source>::get(x); (y,y) }
fn main() { let p=dup::<Wrap<(i32,bool,u64)>,(i32,bool,u64)>(Wrap((42,true,43u64))); if p.0.0==42 { if p.0.1 { if p.0.2==43u64 { if p.1.0==42 { if p.1.1 { if p.1.2==43u64 { println!("ok"); } } } } } } }
"###,
            source_bytes: 493,
            source_fingerprint: 15359024506620092154,
            stock_errors: &[],
            outcome: Outcome::Runtime,
            boundary: Boundary::None,
        },
        // Source SHA256 e41c0d7ae7f1fdc52ac8049c152bd13cc0ed6eda9ed877dfd63948b2513c809f
        Case {
            name: "append_wide_full_payload",
            group: "append",
            source: r###"trait Source: Sized { type Out: Copy; fn get(self) -> <Self as Source>::Out; }
struct Wrap<T>(T);
impl<T: Copy> Source for Wrap<T> { type Out = T; fn get(self) -> <Self as Source>::Out { self.0 } }
fn dup<T: Source<Out = U>, U: Copy>(x: T) -> (U,U) { let y = <T as Source>::get(x); (y,y) }
fn main() { let p=dup::<Wrap<(i64,i64,i64,i64,i64,i64,i64,i64)>,(i64,i64,i64,i64,i64,i64,i64,i64)>(Wrap((51i64,52i64,53i64,54i64,55i64,56i64,57i64,58i64))); if p.0.0==51i64 { if p.0.1==52i64 { if p.0.2==53i64 { if p.0.3==54i64 { if p.0.4==55i64 { if p.0.5==56i64 { if p.0.6==57i64 { if p.0.7==58i64 { if p.1.0==51i64 { if p.1.1==52i64 { if p.1.2==53i64 { if p.1.3==54i64 { if p.1.4==55i64 { if p.1.5==56i64 { if p.1.6==57i64 { if p.1.7==58i64 { println!("ok"); } } } } } } } } } } } } } } } } }
"###,
            source_bytes: 785,
            source_fingerprint: 15560302826484777849,
            stock_errors: &[],
            outcome: Outcome::Runtime,
            boundary: Boundary::None,
        },
        // Source SHA256 d948cae1d55ddd3db77bfea14b8edfca7254cc9a8a3aa939eb586459f51a2cc0
        Case {
            name: "append_duplicate_copy_sized",
            group: "append",
            source: r###"trait Source: Sized { type Out: Copy + Sized + Copy + Sized; fn get(self) -> <Self as Source>::Out; }
struct Wrap<T>(T);
impl<T: Copy> Source for Wrap<T> { type Out = T; fn get(self) -> <Self as Source>::Out { self.0 } }
fn dup<T: Source<Out = U>, U: Copy>(x: T) -> (U,U) { let y = <T as Source>::get(x); (y,y) }
fn main() { let p=dup::<Wrap<i32>,i32>(Wrap(61)); if p.0==61 { if p.1==61 { println!("ok"); } } }
"###,
            source_bytes: 411,
            source_fingerprint: 2728945075207606770,
            stock_errors: &[],
            outcome: Outcome::Runtime,
            boundary: Boundary::None,
        },
        // Source SHA256 e19ec2e1020b1c43523ec302bfaaac0144a4d4335867bf382188f53ccc8889e8
        Case {
            name: "append_explicit_sized_cell",
            group: "append",
            source: r###"trait Source: Sized { type Out: Sized; fn get(self) -> <Self as Source>::Out; }
struct Cell(i32);
struct Item;
impl Source for Item { type Out = Cell; fn get(self) -> <Self as Source>::Out { Cell(71) } }
fn main() { let c=<Item as Source>::get(Item); if c.0==71 { println!("ok"); } }
"###,
            source_bytes: 284,
            source_fingerprint: 4006184707188134394,
            stock_errors: &[],
            outcome: Outcome::Runtime,
            boundary: Boundary::None,
        },
        // Source SHA256 c2217145623e35c965e3bd16bd39fb5b55688ca591eb54962724557d246cb590
        Case {
            name: "append_no_bound_cell",
            group: "append",
            source: r###"trait Source: Sized { type Out; fn get(self) -> <Self as Source>::Out; }
struct Cell(i32);
struct Item;
impl Source for Item { type Out = Cell; fn get(self) -> <Self as Source>::Out { Cell(71) } }
fn main() { let c=<Item as Source>::get(Item); if c.0==71 { println!("ok"); } }
"###,
            source_bytes: 277,
            source_fingerprint: 12692237225503555709,
            stock_errors: &[],
            outcome: Outcome::Runtime,
            boundary: Boundary::None,
        },
        // Source SHA256 8d04e6d08a239346eab1e9bd81bd61a68e02496c6fa87faa9db4f437f775c308
        Case {
            name: "append_unused_missing_t_copy",
            group: "append",
            source: r###"trait Source: Sized { type Out: Copy; fn get(self) -> <Self as Source>::Out; }
struct Wrap<T>(T);
impl<T> Source for Wrap<T> { type Out = T; fn get(self) -> <Self as Source>::Out { self.0 } }
fn main() { println!("ok"); }
"###,
            source_bytes: 222,
            source_fingerprint: 17475812137390099051,
            stock_errors: &["E0277"],
            outcome: Outcome::Invalid,
            boundary: Boundary::TraitObligation,
        },
        // Source SHA256 001320d8ccd1d73fb330dad248bed1b17e621882139dd9e447b7050dc71e2f77
        Case {
            name: "append_unused_cell_copy_rhs",
            group: "append",
            source: r###"trait Source: Sized { type Out: Copy; fn get(self) -> <Self as Source>::Out; }
struct Cell(i32);
struct Item;
impl Source for Item { type Out = Cell; fn get(self) -> <Self as Source>::Out { Cell(71) } }
fn main() { println!("ok"); }
"###,
            source_bytes: 233,
            source_fingerprint: 1030995575941688420,
            stock_errors: &["E0277"],
            outcome: Outcome::Invalid,
            boundary: Boundary::TraitObligation,
        },
        // Source SHA256 ed31ce1255d130fdde96f75c2601abb83db0b2a214f1dafa9481699cca8ca0c7
        Case {
            name: "append_no_u_copy_duplicate",
            group: "append",
            source: r###"trait Source: Sized { type Out: Copy; fn get(self) -> <Self as Source>::Out; }
fn dup<T: Source<Out = U>, U>(x: T) -> (U,U) { let y = <T as Source>::get(x); (y,y) }
fn main() { println!("ok"); }
"###,
            source_bytes: 195,
            source_fingerprint: 14729295356340721756,
            stock_errors: &["E0382"],
            outcome: Outcome::Invalid,
            boundary: Boundary::MoveAnalysis,
        },
        // Source SHA256 8d314edc660dc1b7df3b4e145ec0aa3f8571c144e31e9774f93cb5567baec146
        Case {
            name: "append_wrong_equality",
            group: "append",
            source: r###"trait Source: Sized { type Out: Copy; fn get(self) -> <Self as Source>::Out; }
struct Wrap<T>(T);
impl<T: Copy> Source for Wrap<T> { type Out = T; fn get(self) -> <Self as Source>::Out { self.0 } }
fn dup<T: Source<Out = U>, U: Copy>(x: T) -> (U,U) { let y = <T as Source>::get(x); (y,y) }
fn main() { let _ = dup::<Wrap<i32>,bool>(Wrap(81)); println!("ok"); }
"###,
            source_bytes: 361,
            source_fingerprint: 5708058216300123522,
            stock_errors: &["E0271"],
            outcome: Outcome::Invalid,
            boundary: Boundary::AssociatedEquality,
        },
        // Source SHA256 812e69fc628a39dc686d67275a5ce875e94ff4dff4eaaeed52bdfe359ef4d0d1
        Case {
            name: "append_user_trait_named_copy",
            group: "append",
            source: r###"trait Copy {}
impl Copy for i32 {}
trait Source: Sized { type Out: Copy; fn get(self) -> <Self as Source>::Out; }
struct Wrap<T>(T);
impl<T: Copy> Source for Wrap<T> { type Out = T; fn get(self) -> <Self as Source>::Out { self.0 } }
fn main() { let c=<Wrap<i32> as Source>::get(Wrap(91)); if c==91 { println!("ok"); } }
"###,
            source_bytes: 320,
            source_fingerprint: 9116466779995810242,
            stock_errors: &[],
            outcome: Outcome::Unsupported,
            boundary: Boundary::NominalSelfPattern,
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
            "jr-associated-bounds-{}-{nonce}-{}",
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
            eprintln!("ASSOCIATED BOUNDS CELL FAILURE: {failure}");
            self.0.push(failure);
        } else {
            eprintln!("ASSOCIATED BOUNDS CELL PASS: {label}");
        }
    }
    fn finish(self) {
        assert!(
            self.0.is_empty(),
            "{} associated bounds failures:\n{}",
            self.0.len(),
            self.0.join("\n")
        );
    }
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
    let program = frontend::parse(case.source).expect("frozen positive source parses");
    frontend::check(&program).expect("all bodies check independently of reachability");
    let plan = frontend::PlanCtx::from_program(&program, &ir::PlanBudget::default())
        .expect("real source binders and declared predicates");
    assert_eq!(compiled.source_bytes, case.source.len());
    assert_eq!(compiled.source_files, 1);
    assert_eq!(compiled.executable.is_none(), check);
    assert_eq!(compiled.report.failures, 0);
    assert_eq!(compiled.report.events_dropped, 0);
    assert!(compiled.report.capture_complete);
    let mut source_bodies = 0;
    let mut generic_bodies = 0;
    for index in 0..plan.callable_count() {
        let declaration = plan.callable_decl(index).unwrap();
        evaluated_once(&compiled.report, 110, index, case);
        let universal = match plan.callable_provider(declaration).unwrap() {
            frontend::CallableProvider::Source { .. } => {
                source_bodies += 1;
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
    if case.name.contains("explicit_copy")
        || case.name.contains("payload")
        || case.name.contains("duplicate_copy")
    {
        assert!(
            generic_bodies > 0,
            "{}: generic body binder lost",
            case.name
        );
    }
    assert_eq!(
        compiled
            .report
            .node_evaluations
            .iter()
            .filter(|(key, _)| key.kind == 110)
            .count(),
        plan.callable_count()
    );
    assert_eq!(compiled.control_flow.len(), compiled.analyses.len());
    assert!(compiled.analyses.iter().all(|analysis| analysis.passed()));
    for kind in [102, 108, 109] {
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

fn fingerprint(source: &str) -> u64 {
    source.bytes().fold(0xcbf29ce484222325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
    })
}
#[test]
fn frozen_source_partitions_and_payloads() {
    let cases = cases();
    assert_eq!(cases.len(), 40);
    let mut groups = [0; 4];
    let mut outcomes = [0; 3];
    let mut accepted = 0;
    for case in &cases {
        assert_eq!(case.source.len(), case.source_bytes, "{}", case.name);
        assert_eq!(
            fingerprint(case.source),
            case.source_fingerprint,
            "{}",
            case.name
        );
        assert!(case.source.ends_with('\n'));
        assert!(case.source.contains("println!(\"ok\")"));
        assert!(!case.source.contains("#[allow"));
        assert!(!case.source.contains("#[derive"));
        groups[match case.group {
            "cases" => 0,
            "controls" => 1,
            "supplemental" => 2,
            "append" => 3,
            _ => panic!("unknown group"),
        }] += 1;
        outcomes[match case.outcome {
            Outcome::Runtime => 0,
            Outcome::Invalid => 1,
            Outcome::Unsupported => 2,
        }] += 1;
        accepted += usize::from(case.stock_errors.is_empty());
        if case.group == "append" {
            assert!(case.source.contains("trait Source: Sized"));
            assert!(!case.source.contains("String"));
            assert!(!case.source.contains("Self::Out"));
            assert!(!case.source.contains("T::Out"));
        }
        assert_eq!(
            case.boundary == Boundary::None,
            case.outcome == Outcome::Runtime
        );
    }
    assert_eq!(groups, [18, 8, 3, 11]);
    assert_eq!(outcomes, [8, 14, 18]);
    assert_eq!(accepted, 18);
    let originals: String = cases[..29].iter().map(|c| c.source).collect();
    assert_eq!(fingerprint(&originals), 2831013440210668996);
    let append: String = cases[29..].iter().map(|c| c.source).collect();
    assert_eq!(fingerprint(&append), 704696224508669485);
}
fn diagnostic(case: &Case, text: &str) {
    assert!(!text.is_empty(), "{}: empty diagnostic", case.name);
    assert!(!text.contains("panicked at"), "{}:{text}", case.name);
    // Match the observed phase, not merely any error or any capability failure.
    // File prefixes and session IDs vary between graph and isolated CLI runs.
    let (outcome, expected) = match case.boundary {
        Boundary::MoveAnalysis => (
            Outcome::Invalid,
            "universal generic declaration 0: initialization/move analysis (complete=true): [Diagnostic { kind: ReadNotDefinitelyInitialized, block: BlockId(1), statement: Some(6), origin: Some(ExprId(3)), place: Some(Place { local: LocalId(1), projection: [] }) }]",
        ),
        Boundary::TraitObligation => (
            Outcome::Invalid,
            "trait obligation is not proven by caller bounds or a conditional implementation",
        ),
        Boundary::AssociatedEquality => (
            Outcome::Invalid,
            "associated equality obligation does not match the caller or selected implementation",
        ),
        Boundary::TypeResolution => (Outcome::Unsupported, "unresolved type"),
        Boundary::FunctionResolution => (
            Outcome::Unsupported,
            "in function crate::main: unresolved function",
        ),
        Boundary::Syntax(expected) => (Outcome::Unsupported, expected),
        Boundary::GenericInference => (
            Outcome::Unsupported,
            "unsupported generic inference: insufficient evidence; explicit type arguments required",
        ),
        Boundary::NominalSelfPattern => {
            (Outcome::Unsupported, "impl requires a nominal self pattern")
        }
        Boundary::None => panic!("positive rejected {}:{text}", case.name),
    };
    assert_eq!(case.outcome, outcome, "{}", case.name);
    assert!(
        text.lines().any(|line| line.ends_with(expected)),
        "{}: expected {:?} diagnostic {expected:?}, got:{text}",
        case.name,
        case.boundary
    );
    if matches!(
        case.boundary,
        Boundary::GenericInference | Boundary::AssociatedEquality
    ) {
        assert!(
            text.contains("InvalidMetadata: in callable AstPath {"),
            "{}:{text}",
            case.name
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
    eprintln!(
        "ASSOCIATED BOUNDS ACTUAL STOCK:\n{}",
        String::from_utf8_lossy(&version.stdout)
    );
    let mut failures = CellFailures::default();
    for case in cases() {
        failures.check(format!("stock {}", case.name), || {
            let scratch = Scratch::new();
            scratch.write("client.rs", case.source.as_bytes());
            let image = scratch.path("stock");
            let result = capture(
                Command::new(&rustc)
                    .env_remove("RUSTC_BOOTSTRAP")
                    .args([
                        "--edition=2024",
                        "-Cpanic=abort",
                        "--crate-name=associated_bounds_oracle",
                        "--color=never",
                    ])
                    .arg(scratch.path("client.rs"))
                    .arg("-o")
                    .arg(&image),
                &scratch,
                20,
            );
            assert!(result.stdout.is_empty(), "{}:{result:?}", case.name);
            if case.stock_errors.is_empty() {
                assert_eq!(result.code, 0, "{}:{result:?}", case.name);
                runtime(&image, &scratch, b"ok\n");
            } else {
                assert_eq!(result.code, 1, "{}:{result:?}", case.name);
                let text = String::from_utf8_lossy(&result.stderr);
                let codes: Vec<_> = text
                    .lines()
                    .filter_map(|line| line.strip_prefix("error[")?.split_once(']').map(|v| v.0))
                    .collect();
                assert_eq!(codes, case.stock_errors, "{}:{result:?}", case.name);
                assert!(!image.exists());
            }
        });
    }
    failures.finish();
}
#[test]
fn stock_associated_bounds_exact_frozen_sources() {
    stock_all();
}
fn cli_all() {
    let mut failures = CellFailures::default();
    failures.check("stock prerequisite", stock_all);
    let mut cells = 0;
    let mut runtimes = 0;
    for case in cases() {
        for workers in [1, 8] {
            for check in [true, false] {
                failures.check(
                    format!(
                        "CLI {} workers{workers} check{check} {:?}/{:?}",
                        case.name, case.outcome, case.boundary
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
                            "{}:{result:?}",
                            case.name
                        );
                        if case.outcome != Outcome::Runtime {
                            assert!(result.stdout.is_empty());
                            diagnostic(&case, &String::from_utf8_lossy(&result.stderr));
                        }
                        if case.outcome == Outcome::Runtime && !check {
                            runtime(&image, &scratch, b"ok\n");
                            runtimes += 1;
                        } else {
                            for name in ["native", "client", "a.out"] {
                                assert!(
                                    !scratch.path(name).exists(),
                                    "{}: unexpected {name}",
                                    case.name
                                );
                            }
                        }
                    },
                );
                cells += 1;
            }
        }
    }
    assert_eq!(cells, 160);
    failures.check("runtime count", || assert_eq!(runtimes, 16));
    failures.finish();
    eprintln!(
        "ASSOCIATED BOUNDS CLI COMPLETE:160 classified cells;16 ELF payloads;Runtime8 Invalid14 Unsupported18"
    );
}
fn graph_all() {
    let mut failures = CellFailures::default();
    let mut cells = 0;
    for case in cases() {
        for workers in [1, 8] {
            for check in [true, false] {
                failures.check(
                    format!(
                        "graph {} workers{workers} check{check} {:?}/{:?}",
                        case.name, case.outcome, case.boundary
                    ),
                    || match compiler::compile_observed(case.source.to_owned(), workers, check) {
                        Ok(compiled) => {
                            assert_eq!(case.outcome, Outcome::Runtime, "{}", case.name);
                            successful_all_body_graph(&case, &compiled, check);
                        }
                        Err(failure) => {
                            assert_ne!(case.outcome, Outcome::Runtime, "{}:{failure}", case.name);
                            diagnostic(&case, &failure.to_string());
                            let report = failure.report.expect("rejection retains report");
                            assert!(report.capture_complete);
                            assert_eq!(report.events_dropped, 0);
                            assert!(report.failures > 0);
                            assert!(
                                !report
                                    .node_evaluations
                                    .iter()
                                    .any(|(key, _)| key.kind == 103),
                                "{}: rejected source emitted object",
                                case.name
                            );
                        }
                    },
                );
                cells += 1;
            }
        }
    }
    assert_eq!(cells, 160);
    failures.finish();
    eprintln!(
        "ASSOCIATED BOUNDS GRAPH COMPLETE:160 classified cells;Runtime8 Invalid14 Unsupported18"
    );
}
fn isolated(name: &str, marker: &str, run: fn()) {
    if std::env::var_os(marker).is_some() {
        run();
        return;
    }
    let scratch = Scratch::new();
    let mut command = Command::new(std::env::current_exe().unwrap());
    no_compiler_tools(&mut command, &scratch);
    // The stock prerequisite needs its normal linker/tool search path. Every
    // native CLI invocation still installs no_compiler_tools independently.
    if marker == "JR_ASSOCIATED_BOUNDS_CLI_CHILD" {
        command.env(
            "RUSTC",
            std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()),
        );
        command.env("PATH", std::env::var_os("PATH").unwrap_or_default());
    }
    command
        .args(["--exact", name, "--nocapture"])
        .env(marker, "1");
    let result = capture_bounded(&mut command, &scratch, 240, 8 * 1024 * 1024);
    eprint!("{}", String::from_utf8_lossy(&result.stdout));
    eprint!("{}", String::from_utf8_lossy(&result.stderr));
    assert_eq!(result.code, 0, "{result:?}");
}
#[test]
fn native_associated_bounds_workers1_8_check_build_actual_elf() {
    isolated(
        "native_associated_bounds_workers1_8_check_build_actual_elf",
        "JR_ASSOCIATED_BOUNDS_CLI_CHILD",
        cli_all,
    );
}
#[test]
fn native_associated_bounds_workers1_8_check_build_graph() {
    isolated(
        "native_associated_bounds_workers1_8_check_build_graph",
        "JR_ASSOCIATED_BOUNDS_GRAPH_CHILD",
        graph_all,
    );
}
