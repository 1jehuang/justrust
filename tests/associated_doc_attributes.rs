//! A1 original associated-function docs only, not general attribute support.
//! Stock-first oracle: /usr/bin/rustc 1.98.1, commit
//! 48a229ceaefd4985c50990b14116b6d856af0985, edition2024,
//! x86_64-unknown-linux-gnu, lib metadata, default cfg/profile, 2026-10-19.
//! Warning-accepted doc=42/doc(foo), inner docs and statement docs remain explicit
//! native policy exclusions, not invalid Rust. Associated declaration docs are
//! covered by associated_member_docs.rs.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
use justrust_engine::compiler::{
    self,
    source::{self, SourceErrorKind, SourceOptions},
};
use std::{fs, os::unix::fs::PermissionsExt, process::Command};
#[path = "support/source_macro_binding_oracle.rs"]
mod file_oracle;

#[test]
fn original_generic_inline_and_must_use_function_metadata_matches_stock() {
    let source = r#"
        struct Holder<T>{value:T}
        impl<T> Holder<T>{
            #[inline] #[must_use]
            fn new(value:T)->Holder<T>{Holder{value}}
            #[inline(always)] #[must_use="keep the value"]
            fn take(self)->T where T:Copy{self.value}
        }
        fn main(){let value=Holder::new(7i32).take();assert_eq!(value,7);println!("{}",value);}
    "#;
    file_oracle::Fixture::new(source, &[], None).assert_positive();
    let invalid = "struct Holder<T>{value:T}impl<T> Holder<T>{#[inline]#[must_use]fn unused(value:T)->bool{7u8}}fn main(){}";
    file_oracle::Fixture::new(invalid, &[], None).assert_negative();
}

#[test]
fn original_associated_function_docs_check_build_execute_workers1_8() {
    let text = "trait T { /// required\n fn f(x:i32)->i32; /** default */ fn g(x:i32)->i32{x} } struct S{v:i32} impl T for S { #[doc(hidden)] fn f(x:i32)->i32{x} } impl S { #[doc=\"inherent\"] fn h()->i32{7} } fn main(){assert!(S::h()==7);}";
    for workers in [1, 8] {
        compiler::compile_observed(text.into(), workers, true)
            .unwrap_or_else(|e| panic!("CHECK workers{workers}: {e}"));
        let built = compiler::compile_observed(text.into(), workers, false)
            .unwrap_or_else(|e| panic!("BUILD workers{workers}: {e}"));
        let base = std::env::var_os("JCODE_SCRATCH_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let dir = base.join(format!(
            "jr-associated-doc-{}-{workers}",
            std::process::id()
        ));
        fs::create_dir(&dir).unwrap();
        let binary = dir.join("native");
        fs::write(&binary, built.executable.unwrap()).unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        let output = Command::new(&binary).output().unwrap();
        assert!(
            output.status.success(),
            "{:?}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
        fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn docs_cannot_hide_unused_associated_type_name_or_move_errors() {
    let cases = [
        (
            "trait T { #[doc=\"unused\"] fn f()->u8{true} } fn main(){}",
            "bool",
        ),
        (
            "struct S{v:i32} impl S { /// unused\n fn f()->u8{true} } fn main(){}",
            "bool",
        ),
        (
            "struct S{v:i32} impl S { #[doc(hidden)] fn f()->i32{missing} } fn main(){}",
            "missing",
        ),
        (
            "struct V{v:i32} struct S{v:i32} impl S { #[doc=\"unused\"] fn f(){let a=V{v:1};let b=a;let c=a;} } fn main(){}",
            "move",
        ),
        (
            "trait T { #[doc=\"unused\"] #[inline] fn f()->u8{true} } fn main(){}",
            "bool",
        ),
        (
            "struct S{v:i32} impl S { #[inline(always)] fn f()->i32{missing} } fn main(){}",
            "missing",
        ),
        (
            "struct V{v:i32} struct S{v:i32} impl S { #[inline(never)] fn f(){let a=V{v:1};let b=a;let c=a;} } fn main(){}",
            "move",
        ),
    ];
    for workers in [1, 8] {
        for (text, expected) in cases {
            let program = std::sync::Arc::new(compiler::frontend::parse(text).unwrap());
            let budget = compiler::ir::PlanBudget::default();
            let plan = compiler::frontend::PlanCtx::from_shared_program(program, &budget).unwrap();
            let bad_callable = (0..plan.callable_count())
                .find(|&index| {
                    let info = plan
                        .declaration(plan.callable_decl(index).unwrap())
                        .unwrap();
                    info.name.rsplit("::").next() == Some("f")
                })
                .expect("fixture has an exact named invalid source callable");
            // OBJECT keys of closed roots are callable indices, not
            // declaration ids. Generic trait defaults have no root.
            let bad_root = plan
                .is_closed_root(bad_callable)
                .unwrap()
                .then_some(bad_callable);
            for check_only in [true, false] {
                let failure = match compiler::compile_observed(text.into(), workers, check_only) {
                    Ok(_) => panic!("accepted unused invalid associated body {text}"),
                    Err(failure) => failure,
                };
                assert!(
                    failure.to_string().to_lowercase().contains(expected),
                    "{expected}: {failure}"
                );
                {
                    let report = failure
                        .report
                        .expect("semantic failure retains graph report");
                    assert!(report.capture_complete);
                    assert!(
                        !report.published_kind(105),
                        "failed BUILD published executable"
                    );
                    if check_only {
                        assert!(
                            !report
                                .node_evaluations
                                .iter()
                                .any(|(key, _)| key.kind == 103)
                        );
                    }
                    if let Some(index) = bad_root {
                        for (node, (key, _)) in report.node_ids.iter().zip(&report.node_evaluations)
                        {
                            if key.kind == 103 && key.params == [index as u64] {
                                assert!(
                                    !report.events.iter().any(|event| event.kind == "publish"
                                        && event.node == Some(*node)),
                                    "offending callable published object"
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn stock_valid_forms_outside_docs_only_a1_remain_explicitly_unsupported() {
    for text in [
        "trait T { //! inner\n fn f(); }",
        "struct S{v:i32} impl S { #![doc=\"inner\"] fn f(){} }",
        "trait T { #[doc=42] fn f(); }",
        "trait T { #[doc(foo)] fn f(); }",
        "trait T { #[doc=\"prefix\"] #[inline] fn f(); }",
        "trait T { #[inline] fn f(); }",
        "struct S{v:i32} impl S { #[inline] #[inline(never)] fn f(){} }",
        "struct S{v:i32} impl S { #[inline] #[target_feature(enable=\"sse2\")] unsafe fn f(){} }",
        "struct S{v:i32} impl S { #[target_feature(enable=\"sse2\")] #[inline(always)] unsafe fn f(){} }",
        "#![deny(unused_attributes)] trait T { #[inline] fn f(); }",
        "struct Foo<const N:usize>; trait T { #[inline] fn f()->Foo<{1}>; }",
        "macro_rules! ty {()=>{u8}} trait T { #[inline] fn f()->ty!{}; }",
    ] {
        let error = source::prepare_text("a1.rs", text, &SourceOptions::default()).unwrap_err();
        assert_eq!(error.kind, SourceErrorKind::Unsupported, "{text}: {error}");
    }
}

#[test]
fn original_associated_inline_hints_check_build_execute_without_hint_contract() {
    for (mode, inline) in ["inline", "inline(always)", "inline(never)"]
        .into_iter()
        .enumerate()
    {
        let text = format!(
            "trait T{{#[doc=\"default\"]#[{inline}]fn d(x:i32)->i32{{x}}fn f(x:i32)->i32;}}struct S{{v:i32}}impl T for S{{#[{inline}]fn f(x:i32)->i32{{x}}}}impl S{{/// body\n#[{inline}]fn h()->i32{{7}}}}fn main(){{assert!(S::h()==7);assert!(S::f(3)==3);assert!(S::d(4)==4);}}"
        );
        for workers in [1, 8] {
            compiler::compile_observed(text.clone(), workers, true).unwrap();
            let built = compiler::compile_observed(text.clone(), workers, false).unwrap();
            let base = std::env::var_os("JCODE_SCRATCH_DIR")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(std::env::temp_dir);
            let dir = base.join(format!(
                "jr-associated-inline-{}-{mode}-{workers}",
                std::process::id()
            ));
            fs::create_dir(&dir).unwrap();
            let binary = dir.join("native");
            fs::write(&binary, built.executable.unwrap()).unwrap();
            fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
            let output = Command::new(&binary).output().unwrap();
            assert!(
                output.status.success(),
                "{inline}: {:?}: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
            );
            fs::remove_dir_all(dir).unwrap();
        }
    }
}

#[test]
fn malformed_or_misplaced_docs_are_not_masked_into_valid_programs() {
    for text in [
        "trait T { #[doc=] fn f(); }fn main(){}",
        "trait T { #[doc=\"dangling\"] }fn main(){}",
        "trait T { fn #[doc=\"header\"] f(); }fn main(){}",
        "trait T { fn f(#[doc=\"parameter\"] x:i32); }fn main(){}",
        "trait T { #[doc=\"valid\"] fn f(x:i32 ->i32; }fn main(){}",
        "#[cfg(any())] trait T { #[doc=] fn f(); }fn main(){}",
        "#[cfg(any())] trait T { #[doc=\"\\q\"] fn f(); }fn main(){}",
        "#[cfg(any())] trait T { fn f(){let =;} }fn main(){}",
    ] {
        for workers in [1, 8] {
            assert!(
                compiler::compile_observed(text.into(), workers, true).is_err(),
                "masked invalid placement/syntax: {text}"
            );
        }
    }
    compiler::compile_observed(
        "#[cfg(any())] trait T { #[doc=42] fn f(); }fn main(){}".into(),
        1,
        true,
    )
    .unwrap();
}

#[test]
fn associated_docs_keep_source_span_offsets_and_literal_macro_fences() {
    let text = "trait T{#[doc=\"é\\n\"] fn f();}fn main(){}";
    let prepared = source::prepare_text("utf8.rs", text, &SourceOptions::default()).unwrap();
    assert_eq!(prepared.text.len(), text.len());
    assert_eq!(prepared.attributes.len(), 1);
    assert_eq!(
        prepared.attributes[0].span.lo as usize,
        text.find("doc=").unwrap()
    );
    let literal = "struct S{v:i32}impl S{fn f(){let s=\"#[doc=42] fn fake()\";}}fn main(){}";
    let prepared = source::prepare_text("literal.rs", literal, &SourceOptions::default()).unwrap();
    assert!(prepared.text.contains("#[doc=42] fn fake()"));
    {
        let text = "struct S{v:i32}impl S{m!{#[doc=\"generated\"] fn f(){}}}fn main(){}";
        assert!(
            compiler::compile_observed(text.into(), 1, true).is_err(),
            "generated docs admitted: {text}"
        );
    }
}

#[test]
fn deprecated_metadata_and_global_extern_paths_match_stock() {
    let source = r#"
        use ::std::collections::HashMap;
        struct W<'a> { inner: ::core::slice::Iter<'a, u8> }
        #[deprecated]
        fn old() -> i32 { 1 }
        #[deprecated(since = "1.0.0", note = "use new")]
        struct Legacy;
        struct S;
        impl S {
            #[deprecated = "gone"]
            #[inline]
            fn f(&self) -> i32 { 2 }
            #[deprecated]
            const C: i32 = 3;
        }
        #[allow(deprecated)]
        fn main() {
            let v = [1u8, 2, 3];
            let w = W { inner: v.iter() };
            let mut m: HashMap<i32, i32> = ::std::collections::HashMap::new();
            m.insert(4, ::core::cmp::max(5, 6));
            let _ = Legacy;
            println!("{} {} {} {} {}", old(), S.f(), S::C, w.inner.count(), m[&4]);
        }
    "#;
    file_oracle::Fixture::new(source, &[], None).assert_positive();
    for invalid in [
        "mod foo { pub fn f() {} } fn main() { ::foo::f(); }",
        "fn main() { let _ = ::missing::x(); }",
        "#[deprecated(bogus = \"x\")] fn f() {} fn main() { f(); }",
        "#[deprecated] fn f() -> u8 { true } fn main() {}",
    ] {
        file_oracle::Fixture::new(invalid, &[], None).assert_negative();
    }
}

#[test]
fn stock_valid_cfg_docs_and_field_docs_are_accepted() {
    for text in [
        "struct S{v:i32} impl S { #[cfg(any())] #[doc=\"prefix\"] fn f(){} } fn main(){let _=S{v:1}.v;}",
        "struct S{#[doc=\"field\"]v:i32}fn main(){let _=S{v:1}.v;}",
        "trait T { #[cfg_attr(any(),doc=42)] fn f(); } fn main(){}",
    ] {
        file_oracle::Fixture::new(text, &[], None).assert_positive();
    }
}

#[test]
fn statement_and_field_lint_levels_match_stock() {
    let source = r#"
        struct P {
            #[allow(dead_code)]
            x: i32,
            #[allow(unused)]
            y: i32,
        }
        fn max() -> usize {
            #[allow(deprecated)]
            usize::MAX
        }
        fn main() {
            #[allow(unused_variables)]
            let unused = 1;
            let p = P { x: 2, y: 3 };
            let v = match p.y {
                #[allow(unreachable_patterns)]
                3 => { #[allow(unused, reason = "demo")] 4 }
                _ => 0,
            };
            #[allow(clippy::all)]
            println!("{} {} {}", p.y, v, max() > 1);
        }
    "#;
    file_oracle::Fixture::new(source, &[], None).assert_positive();
    for invalid in [
        "fn main() { #[allow(unused)] let x: u8 = true; }",
        "fn main() { #[allow(bogus::x)] let x = 1; }",
        "fn main() { #[allow = 1] let x = 1; }",
    ] {
        file_oracle::Fixture::new(invalid, &[], None).assert_negative();
    }
    for unsupported in [
        "fn main() { #[deny(unused)] let x = 1; }",
        "fn main() { #[allow(non_camel_case_types)] let x = 1; }",
    ] {
        let error =
            source::prepare_text("lint.rs", unsupported, &SourceOptions::default()).unwrap_err();
        assert_eq!(error.kind, SourceErrorKind::Unsupported, "{unsupported}");
    }
}

#[test]
fn denied_unused_macros_match_stock() {
    let source = r#"
        #![deny(unused_macros)]
        macro_rules! twice { ($e:expr) => { $e * 2 } }
        mod m {
            macro_rules! three { () => { 3 } }
            pub fn f() -> i32 { three!() }
        }
        #[allow(unused_macros)]
        macro_rules! idle { () => {} }
        macro_rules! _quiet { () => {} }
        fn main() {
            println!("{} {}", twice!(4), m::f());
        }
    "#;
    file_oracle::Fixture::new(source, &[], None).assert_positive();
    for invalid in [
        "#![deny(unused_macros)] macro_rules! a { () => {} } fn main() {}",
        "#![forbid(unused_macros)] fn main() { a!() } macro_rules! a { () => {} }",
    ] {
        file_oracle::Fixture::new(invalid, &[], None).assert_negative();
    }
}

#[test]
fn denied_missing_debug_implementations_match_stock() {
    let source = r#"
        #![deny(missing_debug_implementations)]
        use std::fmt;
        #[derive(Debug, Clone)]
        pub struct Derived(pub u8);
        pub struct Manual<T> { pub value: T }
        impl<T: fmt::Display> fmt::Debug for Manual<T> {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "Manual({})", self.value)
            }
        }
        #[derive(Debug)]
        pub enum Kind { A, B }
        struct Private;
        pub(crate) struct Crate;
        mod hidden { pub(crate) struct Inner; }
        fn main() {
            let _ = (Private, Crate, hidden::Inner);
            #[allow(missing_debug_implementations)]
            struct Local;
            let _ = Local;
            println!("{:?} {:?} {:?}", Derived(1), Manual { value: 2 }, Kind::B);
        }
    "#;
    file_oracle::Fixture::new(source, &[], None).assert_positive();
    for invalid in [
        "#![deny(missing_debug_implementations)] pub struct A; fn main() {}",
        "#![forbid(missing_debug_implementations)] pub enum E {} fn main() {}",
        "#![deny(missing_debug_implementations)] pub mod m { pub union U { pub a: u8 } } fn main() {}",
        "#![deny(missing_debug_implementations)] #[derive(Clone)] pub struct C; fn main() {}",
    ] {
        file_oracle::Fixture::new(invalid, &[], None).assert_negative();
    }
    for unsupported in [
        "#![deny(missing_debug_implementations)] mod p { pub struct H; } pub fn h() -> p::H { p::H } fn main() {}",
        "#![deny(missing_debug_implementations)] pub struct S; fn main() { impl std::fmt::Debug for S { fn fmt(&self, _: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { Ok(()) } } }",
    ] {
        let error =
            source::prepare_text("lint.rs", unsupported, &SourceOptions::default()).unwrap_err();
        assert_eq!(error.kind, SourceErrorKind::Unsupported, "{unsupported}");
    }
}

#[test]
fn denied_missing_docs_over_trait_impl_macro_matches_stock() {
    let source = r#"#![deny(missing_docs)]
//! Crate docs.
/// A trait.
pub trait Size {
    /// Size.
    fn size() -> usize;
}
macro_rules! impl_size(
    ($($n:expr),+) => {
        $(
            impl<T> Size for [T; $n] {
                #[inline]
                fn size() -> usize { $n }
            }
        )+
    }
);
impl_size!(1, 2, 3);
fn main() {
    println!("{}", <[u8; 3] as Size>::size());
}
"#;
    file_oracle::Fixture::new(source, &[], None).assert_positive();
    for unsupported in [
        "#![deny(missing_docs)]\n//! c\nmacro_rules! m { () => { pub struct S; } }\nm!();\nfn main() {}",
        "#![deny(missing_docs)]\n//! c\nmacro_rules! m { () => { impl S { pub fn f() {} } } }\n/// S\npub struct S;\nm!();\nfn main() {}",
    ] {
        let error =
            source::prepare_text("lint.rs", unsupported, &SourceOptions::default()).unwrap_err();
        assert_eq!(error.kind, SourceErrorKind::Unsupported, "{unsupported}");
    }
    file_oracle::Fixture::new(
        "#![deny(missing_docs)]\n//! c\nmacro_rules! m { () => { impl Clone for S { fn clone(&self) -> S { S } } } }\npub struct S;\nm!();\nfn main() {}",
        &[],
        None,
    )
    .assert_negative();
}

#[test]
fn inactive_items_lex_with_the_crate_edition() {
    let text = "#[cfg(any())] fn unused(gen: u8) -> u8 { gen } fn main(){}";
    let options = SourceOptions {
        edition: source::Edition::E2021,
        ..SourceOptions::default()
    };
    source::prepare_text("e2021.rs", text, &options).unwrap();
    assert!(source::prepare_text("e2024.rs", text, &SourceOptions::default()).is_err());
}

#[test]
fn separator_free_literal_expr_repetitions_match_stock() {
    let source = r#"
        trait Z { fn z() -> usize; }
        macro_rules! imp {
            ($($len:expr)+) => {$( impl Z for [u8; $len] { fn z() -> usize { $len } } )+}
        }
        imp! { 1 2 3 }
        macro_rules! sum { ($($x:expr)+) => { 0 $(+ $x)+ } }
        fn get<T: Z>() -> usize { T::z() }
        fn main() {
            println!("{} {} {}", get::<[u8; 2]>() + get::<[u8; 3]>(), sum!(1 -2 3), sum!(4));
        }
    "#;
    file_oracle::Fixture::new(source, &[], None).assert_positive();
    file_oracle::Fixture::new(
        "macro_rules! s { ($($x:expr)+) => { 0 $(+ $x)+ } } fn main() { let _: u8 = s!(1 \"a\"); }",
        &[],
        None,
    )
    .assert_negative();
}

#[test]
fn local_impl_member_attributes_pass_source_preparation() {
    // Attribute admission only: parsing local impls is a separate feature.
    let ok = "struct G(u32); trait S { fn s(&self) -> u32; } fn run() -> u32 { \
        impl S for G { #[inline] fn s(&self) -> u32 { self.0 } } \
        impl G { #[cold] fn slow(&self) -> u32 { self.0 } #[cfg(any())] fn gone() { missing() } \
        #[allow(dead_code)] const K: u32 = 5; } 0 } fn main() { run(); }";
    let prepared = source::prepare_text("local.rs", ok, &SourceOptions::default()).unwrap();
    assert!(!prepared.text.contains("missing"));
    for bad in [
        "struct S; fn main() { impl S { #[inline(bogus)] fn f() {} } }",
        "struct S; fn main() { impl S { #[bogus_attr] fn f() {} } }",
    ] {
        assert!(
            source::prepare_text("local.rs", bad, &SourceOptions::default()).is_err(),
            "{bad}"
        );
    }
    // Stock warns (unused inline on a const); natively unsupported, not accepted.
    let error = source::prepare_text(
        "local.rs",
        "struct S; fn main() { impl S { #[inline] const K: u8 = 1; } }",
        &SourceOptions::default(),
    )
    .unwrap_err();
    assert_eq!(error.kind, SourceErrorKind::Unsupported);
}

#[test]
fn parenthesized_macro_rules_bodies_are_payload_match_stock() {
    let source = r#"
        trait Size { fn size() -> usize; }
        macro_rules! impl_size(
            ($($n:expr),+) => {
                $(
                    impl Size for [u8; $n] {
                        #[inline]
                        fn size() -> usize { $n }
                    }
                )+
            }
        );
        impl_size!(1, 4);
        fn get<T: Size>() -> usize { T::size() }
        fn main() { println!("{}", get::<[u8; 1]>() + get::<[u8; 4]>()); }
    "#;
    file_oracle::Fixture::new(source, &[], None).assert_positive();
    file_oracle::Fixture::new(
        "macro_rules! m( () => { fn f() -> u8 { #[inline] true } } ); m!(); fn main() {}",
        &[],
        None,
    )
    .assert_negative();
}

#[test]
fn type_only_arrays_beyond_the_storage_limit_match_stock() {
    let source = r#"
        trait Len { fn len() -> usize; }
        macro_rules! imp { ($($n:expr)+) => {$( impl Len for [u8; $n] { fn len() -> usize { $n } } )+} }
        imp! { 4 33554432 4294967296 }
        fn get<T: Len>() -> usize { T::len() }
        fn main() { println!("{} {}", get::<[u8; 4]>(), get::<[u8; 4294967296]>()); }
    "#;
    file_oracle::Fixture::new(source, &[], None).assert_positive();
    // Materializing such an array stays an explicit native limit.
    assert!(
        compiler::compile_observed(
            "fn main(){ let a = [0u8; 33554432]; let _ = a[0]; }".into(),
            1,
            true
        )
        .is_err()
    );
}

#[test]
fn repr_transparent_single_field_structs_match_stock() {
    let source = r#"
        #[repr(transparent)]
        #[derive(Clone, Copy)]
        struct Meters(u32);
        #[repr(transparent)]
        struct Wrap<T> { inner: T }
        fn main() {
            let m = Meters(7);
            let w = Wrap { inner: m.0 + 1 };
            println!("{} {}", m.0, w.inner);
        }
    "#;
    file_oracle::Fixture::new(source, &[], None).assert_positive();
    for invalid in [
        "#[repr(transparent, C)] struct S(u8); fn main() {}",
        "#[repr(transparent)] fn f() {} fn main() {}",
    ] {
        file_oracle::Fixture::new(invalid, &[], None).assert_negative();
    }
    let error = source::prepare_text(
        "repr.rs",
        "#[repr(transparent)] struct S(u8); fn main() {}",
        &SourceOptions::default(),
    );
    assert!(error.is_ok());
}

#[test]
fn where_shorthand_sees_other_predicates_and_edition_2018_impl_trait_without_lifetimes() {
    // smallvec: a cfg'd-out struct whose where clause uses `T::Item` before
    // (or after) the predicate bounding `T`. Valid syntax in either order.
    for text in [
        "trait Array { type Item; } #[cfg(any())] struct D<T, F> where F: FnMut(&mut T::Item) -> bool, T: Array { t: T, f: F } fn main() {}",
        "trait Array { type Item; } #[cfg(any())] struct D<T, F> where T: Array, F: FnMut(&mut T::Item) -> bool { t: T, f: F } fn main() {}",
    ] {
        compiler::compile_observed(text.into(), 1, true).unwrap_or_else(|e| panic!("{text}: {e}"));
    }
    file_oracle::Fixture::new(
        "trait A { type Item; } struct S<T, F> where F: Fn(&T::Item), T: Clone { t: T, f: F } fn main() {}",
        &[],
        None,
    )
    .assert_negative();
    let options = SourceOptions {
        edition: source::Edition::E2018,
        ..SourceOptions::default()
    };
    let ok = "fn pairs(n: usize) -> impl Iterator<Item = (usize, usize)> { (0..n).map(|i| (i, i + 1)) } fn main() { let _ = pairs(2); }";
    compiler::compile_observed_with_options(ok.into(), 1, true, options.clone()).unwrap();
    // Any possible region input keeps pre-2024 capture unsupported. The
    // elided `Chars` and `W` inputs are rejected by stock --edition 2018;
    // the impl-lifetime cases are valid but must not take the 2024 rule.
    for text in [
        "fn f(x: std::str::Chars) -> impl Iterator<Item = char> { x } fn main() { let s = String::from(\"ab\"); let _ = f(s.chars()).count(); }",
        "struct W<'a>(&'a [u8]); fn f(w: W) -> impl Iterator<Item = u8> { w.0.iter().copied() } fn main() {}",
        "struct W<'a>(&'a [u8]); impl<'a> W<'a> { fn f(self) -> impl Iterator<Item = &'a u8> { self.0.iter() } } fn main() {}",
        "struct W<'a>(&'a [u8]); impl<'a> W<'a> { fn n(&self) -> impl Fn() -> usize { || 3 } } fn main() {}",
        "fn first<'a>(x: &'a [u8]) -> impl Iterator<Item = &'a u8> { x.iter() } fn main() {}",
    ] {
        assert!(
            compiler::compile_observed_with_options(text.into(), 1, true, options.clone()).is_err(),
            "{text}"
        );
    }
}

#[test]
fn body_statement_and_local_item_attributes_match_stock() {
    file_oracle::stock_native(
        r#"
        fn pick(x: u8) -> u8 {
            #[inline(always)]
            fn twice(v: u8) -> u8 { v * 2 }
            /// Local mode type with derived comparisons.
            #[derive(Clone, Copy, PartialEq, Debug)]
            enum Mode { Low, High }
            #[allow(unused_variables)]
            let limit: u8 = 10;
            #[rustfmt::skip]
            let m = if x < limit { Mode::Low } else { Mode::High };
            /// statement documentation
            let doubled = twice(x);
            #[cfg(any())]
            if true { return 99; }
            #[cfg(all())]
            if m == Mode::High { return doubled; }
            #[cfg(any())]
            fn missing_body() -> u8 { not_defined() }
            #[cfg_attr(all(), allow(unused_variables))]
            let unused = m;
            #[cold]
            #[track_caller]
            fn slow() -> u8 { 1 }
            slow() + x
        }
        fn main() { println!("{} {}", pick(3), pick(20)); }
    "#,
    );
    for invalid in [
        "fn f(){ #[inline] struct S; } fn main(){}",
        "fn f(){ #[repr(C)] fn g(){} } fn main(){}",
        "fn f(){ #[inline] let x = 1; } fn main(){}",
        "fn f()->u8{ #[rustfmt::skip] 1 } fn main(){}",
        "fn f(){ #[cfg(all())] fn g()->u8{ 1u16 } } fn main(){}",
        "fn f()->u8{ #[cfg(any())] if true { return 1; } } fn main(){}",
    ] {
        file_oracle::stock_and_native_reject(invalid);
    }
}
fn inactive_unions_are_syntax_only_and_active_unions_stay_explicit() {
    let source = r#"
        #[cfg(any())]
        union Data<A: Copy> { inline: core::mem::ManuallyDrop<A>, heap: (*mut u8, usize) }
        #[cfg(any())]
        impl<A: Copy> Data<A> { fn missing(&self) -> u8 { nothing() } }
        fn main() { let union = 3; println!("{}", union); }
    "#;
    file_oracle::Fixture::new(source, &[], None).assert_positive();
    // Disabled items must still be syntactically valid.
    file_oracle::Fixture::new(
        "#[cfg(any())] union U { a: u8 b: u8 } fn main(){}",
        &[],
        None,
    )
    .assert_negative();
    file_oracle::Fixture::new("#[cfg(any())] union U; fn main(){}", &[], None).assert_negative();
    // An active union is legal Rust that is not implemented: explicit diagnostic.
    let checked =
        compiler::compile_observed("union U { a: u32, b: f32 } fn main(){}".into(), 1, true);
    match checked {
        Ok(_) => panic!("active union accepted"),
        Err(error) => assert!(error.to_string().contains("unsupported"), "{error}"),
    }
}

#[test]
fn shorthand_projections_see_the_functions_later_where_clause() {
    let source = r#"
        trait Seq { type Error; fn fail(&self) -> Self::Error; }
        struct S;
        impl Seq for S { type Error = u16; fn fail(&self) -> u16 { 7 } }
        fn visit<B>(seq: B) -> Result<u8, B::Error> where B: Seq { Err(seq.fail()) }
        fn first<B>(seq: &B, _hint: Option<B::Error>) -> B::Error where B: Seq, { seq.fail() }
        #[cfg(any())]
        impl<'a, T: Arr> Spec<&'a [T::Item]> for W<T> where T::Item: Copy { fn spec(&self, x: &'a [T::Item]) -> usize { x.len() } }
        fn main() { println!("{:?} {}", visit(S), first(&S, None)); }
    "#;
    file_oracle::Fixture::new(source, &[], None).assert_positive();
    // The later bound must still name a trait declaring the member.
    file_oracle::Fixture::new(
        "trait T {} fn f<B>(_: B) -> Option<B::Missing> where B: T { None } fn main(){}",
        &[],
        None,
    )
    .assert_negative();
    file_oracle::Fixture::new(
        "fn f<B>(_: B) -> Option<B::Item> where u8: Copy { None } fn main(){}",
        &[],
        None,
    )
    .assert_negative();
}

#[test]
fn non_exhaustive_positions_match_stock() {
    let source = r#"
        #[non_exhaustive]
        pub struct Config { pub level: u8, #[non_exhaustive] pub name: &'static str }
        #[non_exhaustive]
        #[derive(Debug, Clone, Copy, PartialEq)]
        pub enum Kind { #[non_exhaustive] Unit, #[non_exhaustive] Tuple(i32), Named { #[doc = "x"] x: i64 } }
        #[non_exhaustive] pub(crate) struct Marker;
        fn main() {
            let c = Config { level: 3, name: "n" };
            let Config { level, name } = c;
            let k = Kind::Tuple(4);
            match k { Kind::Unit => println!("u"), Kind::Tuple(v) => println!("{v}"), Kind::Named { x } => println!("{x}") }
            let _ = Marker;
            println!("{level} {name} {:?} {}", Kind::Named { x: 9 }, k == Kind::Unit);
        }
    "#;
    file_oracle::Fixture::new(source, &[], None).assert_positive();
    for invalid in [
        "#[non_exhaustive] fn f(){} fn main(){f()}",
        "#[non_exhaustive] type A = u8; fn main(){let _: A = 1;}",
        "#[non_exhaustive(x)] struct T; fn main(){let _ = T;}",
        "#[non_exhaustive = \"x\"] enum Q { A } fn main(){let _ = Q::A;}",
        "enum Q { #[non_exhaustive(y)] A } fn main(){let _ = Q::A;}",
        "trait Tr {} #[non_exhaustive] impl Tr for u8 {} fn main(){}",
        "mod m { #![non_exhaustive] } fn main(){}",
        "#[non_exhaustive] struct T { a: u8 } fn main(){ let T { a } = T { a: 1u16 }; let _ = a; }",
    ] {
        file_oracle::stock_and_native_reject(invalid);
    }
}

#[test]
fn denied_unconditional_recursion_matches_stock() {
    let source = r#"
        #![deny(unconditional_recursion)]
        pub enum E { A, B }
        fn count(n: u32) -> u32 { if n == 0 { 0 } else { 1 + count(n - 1) } }
        fn stops(e: E) -> u8 { match e { E::A => stops(E::B), E::B => 2 } }
        fn diverges(b: bool) -> u8 { if b { panic!("stop") } diverges(b) }
        fn looped(n: u8) -> u8 { loop { if n > 3 { return n; } return looped(n + 1); } }
        fn exits_early(x: Option<u8>) -> u8 { let Some(v) = x else { return 0 }; exits_early(Some(v).filter(|v| *v > 9)) }
        fn short(b: bool) -> bool { b && short(!b) }
        fn labeled(x: u8) -> u8 { 'a: { if x == 0 { break 'a } return labeled(x - 1); } 7 }
        struct S;
        impl S { fn depth(&self, n: u8) -> u8 { if n == 0 { 0 } else { self.depth(n - 1) + 1 } } }
        trait Tr { fn m(&self) -> u8; }
        struct W<T>(T);
        impl Tr for u8 { fn m(&self) -> u8 { *self } }
        impl<T: Tr> Tr for W<T> { fn m(&self) -> u8 { self.0.m() + 1 } }
        fn main() {
            println!("{} {} {} {} {} {}", count(3), stops(E::A), 0, looped(1), exits_early(Some(4)), short(true));
            println!("{} {} {}", labeled(2), S.depth(3), W(W(4u8)).m());
        }
    "#;
    file_oracle::Fixture::new(source, &[], None).assert_positive();
    for invalid in [
        "#![deny(unconditional_recursion)] fn f() { f() } fn main() {}",
        "#![deny(unconditional_recursion)] fn f(b: bool) { if b { f(b) } else { f(!b) } } fn main() {}",
        "#![deny(unconditional_recursion)] fn f(n: u32) -> u32 { n + f(n) } fn main() {}",
        "#![deny(unconditional_recursion)] fn f() { loop { f() } } fn main() {}",
        "#![deny(unconditional_recursion)] fn f(x: u8) { match x { 0 => f(1), _ => f(0) } } fn main() {}",
        "#![deny(unconditional_recursion)] fn f(x: Option<u8>) { if let Some(_) = x {} f(x) } fn main() {}",
        "#![deny(unconditional_recursion)] struct S; impl S { fn g(&self) { self.g() } } fn main() {}",
        "#![deny(unconditional_recursion)] trait T { fn g(&self) { self.g() } } fn main() {}",
        "#![deny(unconditional_recursion)] fn f<T>(x: T) { f(x) } fn main() {}",
        "#![deny(unconditional_recursion)] fn f(x: u8) { println!(\"{x}\"); f(x) } fn main() {}",
    ] {
        file_oracle::stock_and_native_reject(invalid);
    }
}
