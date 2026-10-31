//! Universal generic templates, unchanged frozen stock28 corpus.
//! Resource errors are separate from invalid Rust and unsupported valid sources.
//! Stock expanding-instance CHECK succeeds, BUILD hits recursion limit; native
//! CHECK/BUILD must reject usefully with exact exit1 and a generic limit diagnostic.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[path = "support/stock_rustc.rs"]
mod defaults_stock;

const SYNC_VALID: &[(&str, &str)] = &[
    (
        "projection_auto_sync",
        r#"trait Family{type Item;}struct W<T:Family>{x:T::Item}fn need<T:Sync>(){}fn forward<T:Family>() where T::Item:Sync{need::<W<T>>();}fn main(){println!("7");}"#,
    ),
    (
        "scopeguard_original_impl_shape_universal",
        r#"trait Strategy{}struct ScopeGuard<T,F:FnOnce(T),S:Strategy>{value:T,dropfn:F,strategy:core::marker::PhantomData<S>}unsafe impl<T,F,S> Sync for ScopeGuard<T,F,S> where T:Sync,F:FnOnce(T),S:Strategy{}fn need<T:Sync>(){}fn forward<T:Sync,F:FnOnce(T),S:Strategy>(){need::<ScopeGuard<T,F,S>>();}fn main(){println!("7");}"#,
    ),
    (
        "explicit_conditional_marked",
        r#"trait Mark{}struct Marked;impl Mark for Marked{}struct W<T>{x:T}unsafe impl<T:Mark> Sync for W<T>{}fn need<T:Sync>(){}fn main(){need::<W<Marked>>();println!("7");}"#,
    ),
    (
        "universal_bound",
        r#"fn need<T:Sync>(){}fn forward<T:Sync>(){need::<T>();}fn main(){forward::<i32>();println!("7");}"#,
    ),
    (
        "conditional_nominal",
        r#"struct W<T>{x:T}fn need<T:Sync>(){}fn forward<T:Sync>(){need::<W<T>>();}fn main(){forward::<i32>();println!("7");}"#,
    ),
    (
        "explicit_impl_does_not_require_f_sync",
        r#"struct Guard<T,F>{value:T,action:F}unsafe impl<T:Sync,F> Sync for Guard<T,F>{}fn need<T:Sync>(){}fn forward<T:Sync,F>(){need::<Guard<T,F>>();}fn main(){forward::<i32,core::cell::Cell<i32>>();println!("7");}"#,
    ),
    (
        "shared_reference_exact_t",
        r#"fn need<T:Sync>(){}fn forward<T:Sync>(x:&T){need::<&T>();}fn main(){let x=7;forward(&x);println!("7");}"#,
    ),
    (
        "manually_drop_exact_t",
        r#"fn need<T:Sync>(){}fn forward<T:Sync>(){need::<core::mem::ManuallyDrop<T>>();}fn main(){forward::<i32>();println!("7");}"#,
    ),
    (
        "phantom_data_exact_t",
        r#"fn need<T:Sync>(){}fn forward<T:Sync>(){need::<core::marker::PhantomData<T>>();}fn main(){forward::<i32>();println!("7");}"#,
    ),
    (
        "tuple_array_nested",
        r#"fn need<T:Sync>(){}fn forward<T:Sync>(){need::<(T,[T;2])>();}fn main(){forward::<i32>();println!("7");}"#,
    ),
];
const SYNC_INVALID: &[(&str, &str)] = &[
    (
        "explicit_conditional_suppresses_auto",
        "trait Mark{}struct W<T>{x:T}unsafe impl<T:Mark> Sync for W<T>{}fn need<T:Sync>(){}fn main(){need::<W<i32>>();}",
    ),
    (
        "missing_universal_bound",
        "fn need<T:Sync>(){}fn bad<T>(){need::<T>();}fn main(){}",
    ),
    (
        "explicit_impl_still_requires_t_sync",
        "struct Guard<T,F>{value:T,action:F}unsafe impl<T:Sync,F> Sync for Guard<T,F>{}fn need<T:Sync>(){}fn main(){need::<Guard<core::cell::Cell<i32>,i32>>();}",
    ),
    (
        "nominal_cell_field",
        "struct W<T>{x:T}fn need<T:Sync>(){}fn main(){need::<W<core::cell::Cell<i32>>>();}",
    ),
    (
        "shared_reference_cell",
        "fn need<T:Sync>(){}fn main(){need::<&core::cell::Cell<i32>>();}",
    ),
    (
        "manually_drop_cell",
        "fn need<T:Sync>(){}fn main(){need::<core::mem::ManuallyDrop<core::cell::Cell<i32>>>();}",
    ),
    (
        "phantom_data_cell",
        "fn need<T:Sync>(){}fn main(){need::<core::marker::PhantomData<core::cell::Cell<i32>>>();}",
    ),
    (
        "phantom_data_raw_pointer",
        "fn need<T:Sync>(){}fn main(){need::<core::marker::PhantomData<*const i32>>();}",
    ),
    (
        "mutable_raw_pointer",
        "fn need<T:Sync>(){}fn main(){need::<*mut i32>();}",
    ),
    (
        "missing_phantom_universal_bound",
        "fn need<T:Sync>(){}fn bad<T>(){need::<core::marker::PhantomData<T>>();}fn main(){}",
    ),
];

const SYNC_UNSUPPORTED: &[(&str, &str)] = &[
    (
        "scopeguard_original_impl_shape",
        r#"trait Strategy{}struct Always;impl Strategy for Always{}struct ScopeGuard<T,F:FnOnce(T),S:Strategy>{value:T,dropfn:F,strategy:core::marker::PhantomData<S>}unsafe impl<T,F,S> Sync for ScopeGuard<T,F,S> where T:Sync,F:FnOnce(T),S:Strategy{}fn drop_value(x:i32){}fn need<T:Sync>(){}fn main(){need::<ScopeGuard<i32,fn(i32),Always>>();println!("7");}"#,
    ),
    (
        "recursive_auto_sync",
        r#"struct R<'a>{next:&'a R<'a>}fn need<T:Sync>(){}fn forward<'a>(){need::<R<'a>>();}fn main(){println!("7");}"#,
    ),
    (
        "dyn_auto_sync",
        r#"trait V{}fn need<T:Sync+?Sized>(){}fn main(){need::<dyn V+Sync>();println!("7");}"#,
    ),
];

#[test]
fn sync_exact_stock_check_build_run() {
    let rustc = defaults_stock::stock_rustc();
    for (valid, cases) in [
        (true, SYNC_VALID),
        (false, SYNC_INVALID),
        (true, SYNC_UNSUPPORTED),
    ] {
        for (label, source) in cases {
            let scratch = Scratch::new();
            let input = scratch.source(source);
            for check in [true, false] {
                let output = scratch.path(if check {
                    "stock.rmeta"
                } else {
                    "stock-program"
                });
                let mut command = Command::new(&rustc);
                command
                    .arg(&input)
                    .args(["--edition=2024", "--crate-name=sync_oracle"]);
                if check {
                    command.arg("--emit=metadata");
                }
                let result = capture(command.arg("-o").arg(&output), &scratch, 20);
                assert_eq!(
                    result.status.success(),
                    valid,
                    "{label} check={check}: {result:?}"
                );
                if valid && !check {
                    let result = capture(&mut Command::new(&output), &scratch, 5);
                    assert!(result.status.success(), "{label}: {result:?}");
                    assert_eq!(result.stdout, b"7\n", "{label}");
                }
                if !valid {
                    assert!(!output.exists(), "{label}: invalid stock output");
                }
            }
        }
    }
}

#[test]
fn sync_native_check_build_run_workers_1_8() {
    for (label, source) in SYNC_VALID {
        eprintln!("Sync valid: {label}");
        runs(source, "7\n");
    }
    for (label, source) in SYNC_INVALID {
        rejects(label, source);
    }
    for (label, source) in SYNC_UNSUPPORTED {
        for workers in [1, 8] {
            let scratch = Scratch::new();
            let input = scratch.source(source);
            let output = scratch.path("unsupported-output");
            for check in [true, false] {
                let result = capture(&mut compiler(&input, &output, workers, check), &scratch, 20);
                assert_eq!(result.status.code(), Some(1), "{label}: {result:?}");
                let diagnostic = String::from_utf8_lossy(&result.stderr);
                assert!(diagnostic.contains("unsupported"), "{label}: {diagnostic}");
                if *label == "scopeguard_original_impl_shape" {
                    assert!(
                        diagnostic.contains("function pointer callable traits require a native indirect Rust ABI implementation"),
                        "{label}: {diagnostic}"
                    );
                }
                assert!(!diagnostic.contains("panicked at"), "{label}: {diagnostic}");
                assert!(!output.exists(), "{label}: unsupported output");
            }
        }
    }
}

const MANUALLY_DROP_VALID: &[(&str, &str)] = &[
    (
        "canonical_core_std",
        r#"fn take(x:core::mem::ManuallyDrop<i32>)->i32{std::mem::ManuallyDrop::into_inner(x)}fn main(){println!("{}",take(core::mem::ManuallyDrop::new(7)));}"#,
    ),
    (
        "imported_alias_context",
        r#"use std::mem::ManuallyDrop as M;fn main(){let x:M<i32>=M::new(7);println!("{}",M::into_inner(x));}"#,
    ),
    (
        "universal_owned_argument",
        r#"use core::mem::ManuallyDrop;fn through<T>(x:T)->T{ManuallyDrop::into_inner(ManuallyDrop::new(x))}fn main(){println!("{}",through(7));}"#,
    ),
    (
        "noncopy_owned_payload",
        r#"use core::mem::ManuallyDrop;struct Token{n:i32}fn main(){let x=ManuallyDrop::new(Token{n:7});let y=ManuallyDrop::into_inner(x);println!("{}",y.n);}"#,
    ),
    (
        "conditional_copy",
        r#"use core::mem::ManuallyDrop;fn twice<T:Copy>(x:ManuallyDrop<T>)->(ManuallyDrop<T>,ManuallyDrop<T>){(x,x)}fn main(){let pair=twice(ManuallyDrop::new(7));println!("{}",ManuallyDrop::into_inner(pair.0));}"#,
    ),
    (
        "shared_payload_projection",
        r#"use core::mem::ManuallyDrop;fn read(x:&ManuallyDrop<i32>)->i32{**x}fn main(){let x=ManuallyDrop::new(7);println!("{}",read(&x));}"#,
    ),
    (
        "mutable_payload_projection",
        r#"use core::mem::ManuallyDrop;fn main(){let mut x=ManuallyDrop::new(1);*x=7;println!("{}",*x);}"#,
    ),
    (
        "nested_payload_field",
        r#"use core::mem::ManuallyDrop;struct Guard{value:ManuallyDrop<i32>}fn main(){let mut x=ManuallyDrop::new(Guard{value:ManuallyDrop::new(1)});*x.value=7;println!("{}",ManuallyDrop::into_inner(x.value));}"#,
    ),
    (
        "suppresses_implicit_payload_drop",
        r#"use core::mem::ManuallyDrop;struct Token;impl Drop for Token{fn drop(&mut self){println!("bad");}}fn main(){let x=ManuallyDrop::new(Token);println!("7");}"#,
    ),
    (
        "explicit_unsafe_payload_drop",
        r#"use core::mem::ManuallyDrop;struct Token;impl Drop for Token{fn drop(&mut self){println!("7");}}fn main(){let mut x=ManuallyDrop::new(Token);unsafe{ManuallyDrop::drop(&mut x);}}"#,
    ),
    (
        "into_inner_restores_payload_cleanup",
        r#"use core::mem::ManuallyDrop;struct Token;impl Drop for Token{fn drop(&mut self){println!("7");}}fn main(){let x=ManuallyDrop::new(Token);let y=ManuallyDrop::into_inner(x);}"#,
    ),
    (
        "source_name_shadow",
        r#"struct ManuallyDrop{n:i32}fn main(){let x=ManuallyDrop{n:7};println!("{}",x.n);}"#,
    ),
];
const MANUALLY_DROP_INVALID: &[(&str, &str)] = &[
    (
        "unsafe_drop_unused_safe_body",
        "use core::mem::ManuallyDrop;fn bad(x:&mut ManuallyDrop<i32>){ManuallyDrop::drop(x);}fn main(){}",
    ),
    (
        "wrong_borrow_kind",
        "use core::mem::ManuallyDrop;fn bad(x:&ManuallyDrop<i32>){unsafe{ManuallyDrop::drop(x);}}fn main(){}",
    ),
    (
        "noncopy_repeated_into_inner",
        "use core::mem::ManuallyDrop;struct Token;fn main(){let x=ManuallyDrop::new(Token);let a=ManuallyDrop::into_inner(x);let b=ManuallyDrop::into_inner(x);}",
    ),
    (
        "noncopy_payload_deref_move",
        "use core::mem::ManuallyDrop;struct Token;fn main(){let x=ManuallyDrop::new(Token);let a=*x;}",
    ),
    (
        "copy_requires_payload_copy",
        "use core::mem::ManuallyDrop;struct Token;fn bad(x:ManuallyDrop<Token>){let a=x;let b=x;}fn main(){}",
    ),
    ("not_prelude", "fn bad(x:ManuallyDrop<i32>){}fn main(){}"),
    (
        "missing_type_actual",
        "fn bad(x:core::mem::ManuallyDrop){}fn main(){}",
    ),
    (
        "wrong_context_actual",
        "use core::mem::ManuallyDrop;fn bad(){let x:ManuallyDrop<bool>=ManuallyDrop::new(7);}fn main(){}",
    ),
    (
        "private_storage_constructor",
        "fn main(){let x=core::mem::ManuallyDrop(7);}",
    ),
    (
        "wrong_value_arity",
        "fn bad(){let x=core::mem::ManuallyDrop::new(1,2);}fn main(){}",
    ),
];
const MANUALLY_DROP_UNSUPPORTED: &[(&str, &str)] = &[
    (
        "unsized_payload",
        r#"fn bad(x:&core::mem::ManuallyDrop<str>){}fn main(){println!("7");}"#,
    ),
    (
        "first_class_new",
        r#"fn main(){let f=core::mem::ManuallyDrop::<i32>::new;let x=f(7);println!("{}",core::mem::ManuallyDrop::into_inner(x));}"#,
    ),
    (
        "executable_clone",
        r#"fn main(){let x=core::mem::ManuallyDrop::new(7);let y=x.clone();println!("{}",*y);}"#,
    ),
];

const PHANTOM_DATA_VALID: &[(&str, &str)] = &[
    (
        "clone_from_nonclone_argument",
        r#"use core::marker::PhantomData;struct Token;fn reset<T>(x:&mut PhantomData<T>,y:&PhantomData<T>){x.clone_from(y)}fn main(){let mut x=PhantomData::<Token>;let y=PhantomData::<Token>;reset(&mut x,&y);println!("7");}"#,
    ),
    (
        "clone_canonical_marker_provider",
        r#"use core::marker::PhantomData;fn main(){let x=PhantomData::<i32>;let y=x.clone();println!("7");}"#,
    ),
    (
        "clone_nonclone_argument",
        r#"use core::marker::PhantomData;struct Token;fn copy<T>(x:PhantomData<T>)->PhantomData<T>{x.clone()}fn main(){let x=copy::<Token>(PhantomData);println!("7");}"#,
    ),
    (
        "canonical_core_std",
        r#"fn take(x:core::marker::PhantomData<i32>){}fn main(){take(std::marker::PhantomData::<i32>);println!("7");}"#,
    ),
    (
        "imported_alias_context",
        r#"use core::marker::PhantomData as Renamed;fn take(x:Renamed<i32>){}fn main(){let x:Renamed<i32>=Renamed;take(x);println!("7");}"#,
    ),
    (
        "scopeguard_shaped_field",
        r#"use core::marker::PhantomData;struct Guard<T,S>{value:T,strategy:PhantomData<S>}fn take(x:Guard<i32,bool>)->i32{x.value}fn main(){println!("{}",take(Guard::<i32,bool>{value:7,strategy:PhantomData}));}"#,
    ),
    (
        "universal_sized_argument",
        r#"use core::marker::PhantomData;fn marker<T>()->PhantomData<T>{PhantomData}fn main(){let x=marker::<bool>();println!("7");}"#,
    ),
    (
        "unconditional_copy_noncopy_argument",
        r#"use core::marker::PhantomData;struct Token;fn twice<T>(x:PhantomData<T>)->(PhantomData<T>,PhantomData<T>){(x,x)}fn main(){let pair=twice::<Token>(PhantomData);println!("7");}"#,
    ),
    (
        "uninhabited_argument_not_runtime_payload",
        r#"use core::marker::PhantomData;enum Void{}fn take(x:PhantomData<Void>){}fn main(){take(PhantomData::<Void>);println!("7");}"#,
    ),
    (
        "source_struct_same_spelling",
        r#"struct PhantomData{value:i32}fn take(x:PhantomData)->i32{x.value}fn main(){println!("{}",take(PhantomData{value:7}));}"#,
    ),
];
const PHANTOM_DATA_INVALID: &[(&str, &str)] = &[
    (
        "clone_from_aliasing_source",
        r#"use core::marker::PhantomData;fn main(){let mut x=PhantomData::<i32>;let source=&x;x.clone_from(source);}"#,
    ),
    (
        "clone_from_wrong_actual",
        r#"use core::marker::PhantomData;fn main(){let mut x=PhantomData::<i32>;let y=PhantomData::<bool>;x.clone_from(&y);}"#,
    ),
    (
        "not_implicit_prelude",
        "fn bad(x:PhantomData<i32>){}fn main(){}",
    ),
    (
        "unimported_constructor",
        "fn main(){let x=PhantomData::<i32>;}",
    ),
    (
        "missing_type_actual",
        "fn bad(x:core::marker::PhantomData){}fn main(){}",
    ),
    (
        "too_many_actuals",
        "fn bad(x:core::marker::PhantomData<i32,bool>){}fn main(){}",
    ),
    (
        "wrong_context_actual",
        "use core::marker::PhantomData;fn bad(){let x:PhantomData<i32>=PhantomData::<bool>;}fn main(){}",
    ),
    (
        "parenthesized_not_function",
        "fn main(){let x=core::marker::PhantomData::<i32>();}",
    ),
];
const PHANTOM_DATA_UNSUPPORTED: &[(&str, &str)] = &[
    (
        "unsized_argument",
        r#"fn bad(x:core::marker::PhantomData<str>){}fn main(){println!("7");}"#,
    ),
    (
        "borrowed_argument",
        r#"fn bad(x:core::marker::PhantomData<&'static i32>){}fn main(){println!("7");}"#,
    ),
];

#[test]
fn manually_drop_exact_stock_baseline() {
    let rustc = defaults_stock::stock_rustc();
    for (label, source) in MANUALLY_DROP_VALID.iter().chain(MANUALLY_DROP_UNSUPPORTED) {
        let scratch = Scratch::new();
        let input = scratch.source(source);
        for check in [true, false] {
            let output = scratch.path(if check {
                "stock.rmeta"
            } else {
                "stock-program"
            });
            let mut command = Command::new(&rustc);
            command
                .arg(&input)
                .args(["--edition=2024", "--crate-name=manually_drop"]);
            if check {
                command.arg("--emit=metadata");
            }
            let result = capture(command.arg("-o").arg(&output), &scratch, 20);
            assert!(result.status.success(), "{label} check{check}: {result:?}");
            if !check {
                let result = capture(&mut Command::new(&output), &scratch, 5);
                assert!(result.status.success(), "{label}: {result:?}");
                assert_eq!(result.stdout, b"7\n", "{label}");
            }
        }
    }
    for (label, source) in MANUALLY_DROP_INVALID {
        let scratch = Scratch::new();
        let input = scratch.source(source);
        for check in [true, false] {
            let output = scratch.path(if check {
                "invalid.rmeta"
            } else {
                "invalid-program"
            });
            let mut command = Command::new(&rustc);
            command
                .arg(&input)
                .args(["--edition=2024", "--crate-name=manually_drop"]);
            if check {
                command.arg("--emit=metadata");
            }
            let result = capture(command.arg("-o").arg(&output), &scratch, 20);
            assert!(
                !result.status.success(),
                "{label}: stock accepted check{check}"
            );
            assert!(!output.exists(), "{label}: invalid stock output");
        }
    }
}

#[test]
fn manually_drop_native_check_build_run_workers_1_8() {
    for (label, source) in MANUALLY_DROP_VALID {
        eprintln!("ManuallyDrop valid: {label}");
        runs(source, "7\n");
    }
    for (label, source) in MANUALLY_DROP_INVALID {
        rejects(label, source);
    }
    for (label, source) in MANUALLY_DROP_UNSUPPORTED {
        rejects(label, source);
        for workers in [1, 8] {
            let scratch = Scratch::new();
            let input = scratch.source(source);
            let output = scratch.path("unsupported-output");
            for check in [true, false] {
                let result = capture(&mut compiler(&input, &output, workers, check), &scratch, 20);
                let diagnostic = String::from_utf8_lossy(&result.stderr);
                assert!(diagnostic.contains("unsupported:"), "{label}: {diagnostic}");
                assert!(!output.exists(), "{label}: unsupported output");
            }
        }
    }
}

#[test]
fn phantom_data_exact_stock_baseline() {
    let rustc = defaults_stock::stock_rustc();
    for (label, source) in PHANTOM_DATA_VALID.iter().chain(PHANTOM_DATA_UNSUPPORTED) {
        let scratch = Scratch::new();
        let input = scratch.source(source);
        for check in [true, false] {
            let output = scratch.path(if check {
                "stock.rmeta"
            } else {
                "stock-program"
            });
            let mut command = Command::new(&rustc);
            command
                .arg(&input)
                .args(["--edition=2024", "--crate-name=phantom_data"]);
            if check {
                command.arg("--emit=metadata");
            }
            let result = capture(command.arg("-o").arg(&output), &scratch, 20);
            assert!(result.status.success(), "{label} check{check}: {result:?}");
            if !check {
                let result = capture(&mut Command::new(&output), &scratch, 5);
                assert!(result.status.success(), "{label}: {result:?}");
                assert_eq!(result.stdout, b"7\n", "{label}");
            }
        }
    }
    for (label, source) in PHANTOM_DATA_INVALID {
        let scratch = Scratch::new();
        let input = scratch.source(source);
        for check in [true, false] {
            let output = scratch.path(if check {
                "invalid.rmeta"
            } else {
                "invalid-program"
            });
            let mut command = Command::new(&rustc);
            command
                .arg(&input)
                .args(["--edition=2024", "--crate-name=phantom_data"]);
            if check {
                command.arg("--emit=metadata");
            }
            let result = capture(command.arg("-o").arg(&output), &scratch, 20);
            assert!(
                !result.status.success(),
                "{label}: stock accepted check{check}"
            );
            assert!(!output.exists(), "{label}: invalid stock output");
        }
    }
}

#[test]
fn phantom_data_native_check_build_run_workers_1_8() {
    for (label, source) in PHANTOM_DATA_VALID {
        eprintln!("PhantomData valid: {label}");
        runs(source, "7\n");
    }
    for (label, source) in PHANTOM_DATA_INVALID {
        rejects(label, source);
    }
    for (label, source) in PHANTOM_DATA_UNSUPPORTED {
        rejects(label, source);
        for workers in [1, 8] {
            let scratch = Scratch::new();
            let input = scratch.source(source);
            let output = scratch.path("unsupported-output");
            for check in [true, false] {
                let result = capture(&mut compiler(&input, &output, workers, check), &scratch, 20);
                let diagnostic = String::from_utf8_lossy(&result.stderr);
                assert!(diagnostic.contains("unsupported:"), "{label}: {diagnostic}");
                assert!(!output.exists(), "{label}: unsupported output");
            }
        }
    }
}

const NOMINAL_DEFAULT_VALID: &[(&str, &str, &str)] = &[
    (
        "struct_annotation",
        r#"struct Cell<T=i32>{value:T}fn get(x:Cell)->i32{x.value}fn main(){println!("{}",get(Cell::<i32>{value:7}));}"#,
        "7
",
    ),
    (
        "explicit_override_and_inference",
        r#"struct Cell<T=i32>{value:T}fn get(x:Cell<bool>)->bool{x.value}fn main(){let x=Cell{value:true};if get(x){println!("defaults ok");}}"#,
        "defaults ok
",
    ),
    (
        "ordered_partial_arguments",
        r#"struct Pair<A=i32,B=A,C=B>{a:A,b:B,c:C}fn get(x:Pair<i32>)->i32{x.a+x.b+x.c}fn main(){println!("{}",get(Pair::<i32>{a:1,b:2,c:4}));}"#,
        "7
",
    ),
    (
        "tuple_default",
        r#"struct Cell<T=i32>(T);fn get(x:Cell)->i32{x.0}fn main(){println!("{}",get(Cell::<i32>(7)));}"#,
        "7
",
    ),
    (
        "enum_default",
        r#"enum Cell<T=i32>{Value(T),Empty}fn get(x:Cell)->i32{match x{Cell::Value(v)=>v,Cell::Empty=>0}}fn main(){println!("{}",get(Cell::<i32>::Value(7)));}"#,
        "7
",
    ),
    (
        "declaration_module_default",
        r#"mod origin{pub struct Value{pub n:i32}pub struct Cell<T=Value>{pub value:T}}struct Value{n:bool}fn get(x:origin::Cell)->i32{x.value.n}fn main(){println!("{}",get(origin::Cell::<origin::Value>{value:origin::Value{n:7}}));}"#,
        "7
",
    ),
    (
        "closed_bound",
        r#"trait Strategy{}enum Always{}impl Strategy for Always{}struct Guard<T,S:Strategy=Always>{value:T,strategy:S}fn take(x:Guard<i32>)->i32{match x.strategy{}}fn main(){println!("defaults ok");}"#,
        "defaults ok
",
    ),
    (
        "earlier_parameter_declared_proof",
        r#"struct Pair<T:Copy,U:Copy=T>{a:T,b:U}fn get(x:Pair<i32>)->i32{x.a+x.b}fn main(){println!("{}",get(Pair::<i32>{a:3,b:4}));}"#,
        "7
",
    ),
];

const NOMINAL_DEFAULT_UNSUPPORTED: &[(&str, &str)] = &[
    (
        "borrowed_default",
        "struct Cell<T=&'static i32>{value:T}fn main(){}",
    ),
    (
        "pointer_default",
        "struct Cell<T=*const i32>{value:T}fn main(){}",
    ),
    (
        "trait_nonself_default",
        "trait Strategy<T=i32>{}fn main(){}",
    ),
    ("alias_default", "type Cell<T=i32>=T;fn main(){}"),
    (
        "conditional_nominal_default",
        "struct Wrap<T>{value:T}struct Cell<T,U=Wrap<T>>{a:T,b:U}fn main(){}",
    ),
];

const NOMINAL_DEFAULT_INVALID: &[(&str, &str)] = &[
    (
        "unused_recursive_default",
        "struct Bad<T=Bad>{value:T}fn main(){}",
    ),
    (
        "unused_mutually_recursive_defaults",
        "struct First<T=Second>{value:T}struct Second<T=First>{value:T}fn main(){}",
    ),
    (
        "nominal_self_default",
        "struct Bad<T=Self>{value:T}fn main(){}",
    ),
    (
        "unused_unknown_default",
        "struct Bad<T=Missing>{value:T}fn main(){}",
    ),
    (
        "unused_forward_default",
        "struct Bad<T=U,U=i32>{a:T,b:U}fn main(){}",
    ),
    (
        "unused_self_parameter_default",
        "struct Bad<T=T>{value:T}fn main(){}",
    ),
    (
        "nontrailing_default",
        "struct Bad<T=i32,U>{a:T,b:U}fn main(){}",
    ),
    (
        "unused_closed_bound_failure",
        "trait Mark{}struct Other{n:i32}struct Bad<T:Mark=Other>{value:T}fn main(){}",
    ),
    (
        "unused_nested_bound_failure",
        "trait Mark{}struct Inner<T:Mark>{value:T}struct Bad<T=Inner<i32>>{value:T}fn main(){}",
    ),
    (
        "missing_required_argument",
        "struct Pair<T,U=i32>{a:T,b:U}fn bad(x:Pair){}fn main(){}",
    ),
    (
        "extra_argument",
        "struct Cell<T=i32>{value:T}fn bad(x:Cell<i32,bool>){}fn main(){}",
    ),
    (
        "fixed_default_not_field_evidence",
        "struct Pair<T,U=i32>{a:T,b:U}fn main(){let _=Pair::<bool>{a:true,b:false};}",
    ),
];

#[test]
fn nominal_defaults_exact_stock_baseline() {
    let rustc = defaults_stock::stock_rustc();
    for (label, source, expected) in NOMINAL_DEFAULT_VALID.iter().copied().chain(
        NOMINAL_DEFAULT_UNSUPPORTED
            .iter()
            .map(|(label, source)| (*label, *source, "")),
    ) {
        let scratch = Scratch::new();
        let input = scratch.source(source);
        for check in [true, false] {
            let output = scratch.path(if check {
                "check.rmeta"
            } else {
                "stock-program"
            });
            let mut command = Command::new(&rustc);
            command
                .arg(&input)
                .args(["--edition=2024", "--crate-name=nominal_defaults"]);
            if check {
                command.arg("--emit=metadata");
            }
            command.arg("-o").arg(&output);
            let result = capture(&mut command, &scratch, 20);
            assert!(
                result.status.success(),
                "{label}: stock check={check}: {}\n{source}",
                String::from_utf8_lossy(&result.stderr)
            );
            if !check {
                let result = capture(&mut Command::new(&output), &scratch, 5);
                assert!(
                    result.status.success(),
                    "{label}: stock runtime: {result:?}"
                );
                assert_eq!(result.stdout, expected.as_bytes(), "{label}: stock output");
            }
        }
    }
    for (label, source) in NOMINAL_DEFAULT_INVALID {
        let scratch = Scratch::new();
        let input = scratch.source(source);
        for check in [true, false] {
            let output = scratch.path(if check {
                "invalid.rmeta"
            } else {
                "invalid-program"
            });
            let mut command = Command::new(&rustc);
            command
                .arg(&input)
                .args(["--edition=2024", "--crate-name=nominal_defaults"]);
            if check {
                command.arg("--emit=metadata");
            }
            let result = capture(command.arg("-o").arg(&output), &scratch, 20);
            assert!(
                !result.status.success(),
                "{label}: stock accepted check={check}\n{source}"
            );
            assert!(!output.exists(), "{label}: stock published invalid output");
        }
    }
}

#[test]
fn nominal_defaults_native_check_build_run_workers_1_8() {
    for (label, source, expected) in NOMINAL_DEFAULT_VALID {
        eprintln!("nominal default valid: {label}");
        runs(source, expected);
    }
    for (label, source) in NOMINAL_DEFAULT_INVALID {
        rejects(label, source);
    }
    for (label, source) in NOMINAL_DEFAULT_UNSUPPORTED {
        rejects(label, source);
        for workers in [1, 8] {
            let scratch = Scratch::new();
            let input = scratch.source(source);
            let output = scratch.path("unsupported-output");
            for check in [true, false] {
                let result = capture(&mut compiler(&input, &output, workers, check), &scratch, 20);
                let diagnostic = String::from_utf8_lossy(&result.stderr).to_lowercase();
                assert!(
                    diagnostic.contains("unsupported"),
                    "{label}: lost explicit frontier: {diagnostic}"
                );
            }
        }
    }
}

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
            let path = base.join(format!("jr-generics-{}-{epoch}-{id}", std::process::id()));
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

fn runs(source: &str, expected: &str) {
    runs_with_exit(source, expected, 0);
}

fn rejects_resource_build(source: &str) {
    rejects_resource_modes(source, &[false]);
}
fn rejects_resource_modes(source: &str, modes: &[bool]) {
    for workers in [1, 8] {
        let scratch = Scratch::new();
        let input = scratch.source(source);
        let output = scratch.path("must-not-exist");
        for &check in modes {
            let result = capture(&mut compiler(&input, &output, workers, check), &scratch, 20);
            assert_eq!(
                result.status.code(),
                Some(1),
                "resource failure workers={workers}, check={check}: {result:?}"
            );
            let diagnostic = String::from_utf8_lossy(&result.stderr).to_ascii_lowercase();
            assert!(
                diagnostic.contains("generic") && diagnostic.contains("limit"),
                "expected bounded generic limit diagnostic, not arbitrary rejection: {result:?}"
            );
            for forbidden in [
                "panicked at",
                "parse error",
                "unexpected token",
                "unsupported syntax",
            ] {
                assert!(
                    !diagnostic.contains(forbidden),
                    "not a resource guard ({forbidden}): {result:?}"
                );
            }
            assert!(!output.exists(), "resource rejection published executable");
        }
    }
}

#[test]
fn accepts_scalar_identity() {
    runs(
        r#"fn id<T>(x:T)->T{x}fn run()->i32{id::<i32>(7)}
fn main(){if run()==7{println!("generic oracle ok");}}"#,
        "generic oracle ok\n",
    );
}

#[test]
fn accepts_noncopy_identity() {
    runs(
        r#"struct S{x:i32}fn id<T>(x:T)->T{x}fn run()->i32{let s=id::<S>(S{x:7});s.x}
fn main(){if run()==7{println!("generic oracle ok");}}"#,
        "generic oracle ok\n",
    );
}

#[test]
fn accepts_scalar_wrapper() {
    runs(
        r#"struct Wrap<T>{value:T}fn wrap<T>(x:T)->Wrap<T>{Wrap::<T>{value:x}}fn run()->i32{wrap::<i32>(7).value}
fn main(){if run()==7{println!("generic oracle ok");}}"#,
        "generic oracle ok\n",
    );
}

#[test]
fn accepts_noncopy_wrapper() {
    runs(
        r#"struct S{x:i32}struct Wrap<T>{value:T}fn wrap<T>(x:T)->Wrap<T>{Wrap::<T>{value:x}}fn run()->i32{let w=wrap::<S>(S{x:7});w.value.x}
fn main(){if run()==7{println!("generic oracle ok");}}"#,
        "generic oracle ok\n",
    );
}

#[test]
fn accepts_enum_variant_shapes() {
    runs(
        r#"enum E<T>{Unit,Tuple(T),Braced{value:T}}fn get<T>(e:E<T>,fallback:T)->T{match e{E::Unit=>fallback,E::Tuple(v)=>v,E::Braced{value}=>value}}fn run()->i32{let a=get::<i32>(E::<i32>::Unit,1);let b=get::<i32>(E::<i32>::Tuple(2),0);let c=get::<i32>(E::<i32>::Braced{value:4},0);a+b+c}
fn main(){if run()==7{println!("generic oracle ok");}}"#,
        "generic oracle ok\n",
    );
}

#[test]
fn accepts_tuple_payload_match() {
    runs(
        r#"enum E<T>{A((T,i32)),B}fn extract<T>(e:E<T>,other:T)->T{match e{E::A((v,_))=>v,E::B=>other}}fn run()->i32{extract::<i32>(E::<i32>::A((7,2)),0)}
fn main(){if run()==7{println!("generic oracle ok");}}"#,
        "generic oracle ok\n",
    );
}

#[test]
fn accepts_braced_payload_match() {
    runs(
        r#"enum E<T>{A{value:T,n:i32},B}fn extract<T>(e:E<T>,other:T)->T{match e{E::A{value:v,..}=>v,E::B=>other}}fn run()->i32{extract::<i32>(E::<i32>::A{value:7,n:2},0)}
fn main(){if run()==7{println!("generic oracle ok");}}"#,
        "generic oracle ok\n",
    );
}

#[test]
fn accepts_struct_disjoint_partial_moves() {
    runs(
        r#"struct Pair<T>{a:T,b:T}fn split<T>(p:Pair<T>)->(T,T){let a=p.a;let b=p.b;(a,b)}fn run()->i32{let p=split::<i32>(Pair::<i32>{a:3,b:4});p.0+p.1}
fn main(){if run()==7{println!("generic oracle ok");}}"#,
        "generic oracle ok\n",
    );
}

#[test]
fn accepts_singleton_enum_disjoint_moves() {
    runs(
        r#"enum Pair<T>{A{a:T,b:T}}fn split<T>(p:Pair<T>)->(T,T){let Pair::A{a,..}=p;let Pair::A{b,..}=p;(a,b)}fn run()->i32{let p=split::<i32>(Pair::<i32>::A{a:3,b:4});p.0+p.1}
fn main(){if run()==7{println!("generic oracle ok");}}"#,
        "generic oracle ok\n",
    );
}

#[test]
fn accepts_explicit_generic_forwarding() {
    runs(
        r#"fn id<T>(x:T)->T{x}fn forward<U>(x:U)->U{id::<U>(x)}fn run()->i32{forward::<i32>(7)}
fn main(){if run()==7{println!("generic oracle ok");}}"#,
        "generic oracle ok\n",
    );
}

#[test]
fn accepts_exact_self_recursion() {
    runs(
        r#"fn rec<T>(x:T,n:i32)->T{if n==0{x}else{rec::<T>(x,n-1)}}fn run()->i32{rec::<i32>(7,4)}
fn main(){if run()==7{println!("generic oracle ok");}}"#,
        "generic oracle ok\n",
    );
}

#[test]
fn accepts_exact_mutual_recursion() {
    runs(
        r#"fn a<T>(x:T,n:i32)->T{if n==0{x}else{b::<T>(x,n-1)}}fn b<U>(x:U,n:i32)->U{if n==0{x}else{a::<U>(x,n-1)}}fn run()->i32{a::<i32>(7,4)}
fn main(){if run()==7{println!("generic oracle ok");}}"#,
        "generic oracle ok\n",
    );
}

#[test]
fn accepts_multiple_nominal_specializations() {
    runs(
        r#"struct Wrap<T>{value:T}fn id<T>(x:T)->T{x}fn run()->i32{let a=id::<Wrap<i32>>(Wrap::<i32>{value:7});let b=id::<Wrap<u64>>(Wrap::<u64>{value:18446744073709551615u64});if b.value==18446744073709551615u64{a.value}else{0}}
fn main(){if run()==7{println!("generic oracle ok");}}"#,
        "generic oracle ok\n",
    );
}

#[test]
fn accepts_distinct_nominal_generic_owners() {
    runs(
        r#"struct A<T>{value:T}struct B<T>{value:T}fn run()->i32{let a=A::<i32>{value:3};let b=B::<i32>{value:4};a.value+b.value}
fn main(){if run()==7{println!("generic oracle ok");}}"#,
        "generic oracle ok\n",
    );
}

#[test]
fn accepts_nested_reference_free_substitution() {
    runs(
        r#"struct Wrap<T>{value:T}enum E<T>{A(T)}fn id<T>(x:T)->T{x}fn run()->i32{let w=id::<Wrap<E<([i32;1],i32)>>>(Wrap::<E<([i32;1],i32)>>{value:E::<([i32;1],i32)>::A(([3],4))});match w.value{E::A((a,b))=>a[0]+b}}
fn main(){if run()==7{println!("generic oracle ok");}}"#,
        "generic oracle ok\n",
    );
}

#[test]
fn accepts_valid_unused_template() {
    runs(
        r#"fn unused<T>(x:T)->T{x}fn run()->i32{7}
fn main(){if run()==7{println!("generic oracle ok");}}"#,
        "generic oracle ok\n",
    );
}

#[test]
fn rejects_invalid_unused_duplicate() {
    rejects(
        "negative: unused_duplicate",
        r#"fn bad<T>(x:T)->(T,T){(x,x)}fn run()->i32{7}
fn main(){run();}"#,
    );
}

#[test]
fn rejects_invalid_duplicate_with_copy_caller() {
    rejects(
        "negative: duplicate_with_copy_caller",
        r#"fn bad<T>(x:T)->(T,T){(x,x)}fn run()->i32{let p=bad::<i32>(7);p.0}
fn main(){run();}"#,
    );
}

#[test]
fn rejects_invalid_generic_operator_copy_caller() {
    rejects(
        "negative: generic_operator_copy_caller",
        r#"fn bad<T>(x:T)->T{x+1}fn run()->i32{bad::<i32>(7)}
fn main(){run();}"#,
    );
}

#[test]
fn rejects_invalid_generic_return_mismatch() {
    rejects(
        "negative: generic_return_mismatch",
        r#"fn bad<T>(x:T)->T{7i32}fn run()->i32{7}
fn main(){run();}"#,
    );
}

#[test]
fn rejects_invalid_generic_field_reuse() {
    rejects(
        "negative: generic_field_reuse",
        r#"struct Pair<T>{a:T,b:T}fn bad<T>(p:Pair<T>)->(T,T){let a=p.a;(a,p.a)}fn run()->i32{7}
fn main(){run();}"#,
    );
}

#[test]
fn rejects_invalid_whole_after_generic_partial_move() {
    rejects(
        "negative: whole_after_generic_partial_move",
        r#"struct Pair<T>{a:T,b:T}fn bad<T>(p:Pair<T>)->Pair<T>{let a=p.a;p}fn run()->i32{7}
fn main(){run();}"#,
    );
}

#[test]
fn rejects_invalid_nominal_argument_mismatch() {
    rejects(
        "negative: nominal_argument_mismatch",
        r#"struct Wrap<T>{value:T}fn take(x:Wrap<i32>)->i32{x.value}fn run()->i32{take(Wrap::<u8>{value:7})}
fn main(){run();}"#,
    );
}

#[test]
fn rejects_invalid_wrong_generic_arity() {
    rejects(
        "negative: wrong_generic_arity",
        r#"fn id<T>(x:T)->T{x}fn run()->i32{id::<i32,u8>(7)}
fn main(){run();}"#,
    );
}

#[test]
fn runs_copy_bound_trait_dependency() {
    runs(
        r#"fn dup<T:Copy>(x:T)->(T,T){(x,x)}fn run()->i32{let p=dup::<i32>(7);p.0}
fn main(){if run()==7{println!("generic oracle ok");}}"#,
        "generic oracle ok\n",
    );
}

#[test]
fn rejects_legal_unsupported_direct_reference_substitution() {
    // Historical group name retained. The exact source now has native parity evidence.
    runs(
        r#"fn id<T>(x:T)->T{x}fn run()->i32{let x=7;let r=id::<&i32>(&x);*r}
fn main(){if run()==7{println!("generic oracle ok");}}"#,
        "generic oracle ok\n",
    );
}

#[test]
fn rejects_escaping_hidden_reference_substitution() {
    // Stock rustc: E0515, the reference to local `x` escapes through `id`.
    rejects(
        "escaping hidden reference",
        r#"struct Wrap<T>{value:T}fn id<T>(x:T)->T{x}fn run()->&'static i32{let x=7;let w=id::<Wrap<&i32>>(Wrap::<&i32>{value:&x});w.value}
fn main(){if *run()==7{println!("generic oracle ok");}}"#,
    );
}

#[test]
fn accepts_hidden_reference_substitution() {
    // Valid Rust: stock rustc builds and prints the oracle. Per-instance
    // INSTANCE planning now admits it; the escaping variant stays rejected.
    runs(
        r#"struct Wrap<T>{value:T}fn id<T>(x:T)->T{x}fn run()->i32{let x=7;let w=id::<Wrap<&i32>>(Wrap::<&i32>{value:&x});*w.value}
fn main(){if run()==7{println!("generic oracle ok");}}"#,
        "generic oracle ok\n",
    );
}

#[test]
fn rejects_resource_expanding_recursive_instances() {
    use std::io::{BufRead, BufReader, Read};

    // Stock rustc accepts this under `--emit=metadata` (check) and rejects it
    // only while instantiating for codegen ("reached the recursion limit while
    // instantiating"). CHECK must therefore accept, BUILD must reject.
    let source = r#"fn grow<T>(x:T){grow::<(T,)>((x,));}fn run()->i32{grow::<i32>(1);7}
fn main(){if run()==7{println!("generic oracle ok");}}"#;
    for workers in [1, 8] {
        let scratch = Scratch::new();
        let input = scratch.source(source);
        let output = scratch.path("must-not-exist");
        let profile = scratch.path("recursive-check-profile.json");
        let trace = scratch.path("recursive-check-trace.txt");
        let mut command = compiler(&input, &output, workers, true);
        command
            .arg("--profile")
            .arg(&profile)
            .arg("--trace")
            .arg(&trace);
        let result = capture(&mut command, &scratch, 20);
        if !result.status.success() {
            // No JSON dependency is needed: the profile writer places exact
            // first_failure_task (producer node/kind/params) before graph data.
            // Read and print only this bounded prefix, never the full graph.
            let profile_prefix = (|| -> std::io::Result<String> {
                let mut bytes = Vec::new();
                fs::File::open(&profile)?
                    .take(16 * 1024)
                    .read_to_end(&mut bytes)?;
                let text = String::from_utf8_lossy(&bytes);
                Ok(text.split(",\"graph\":").next().unwrap_or(&text).to_owned())
            })();
            eprintln!(
                "recursive CHECK workers={workers} profile prefix (<=16KiB): {profile_prefix:?}"
            );
            // The Debug trace has a distinct node_errors section. Bound both
            // scanning (64MiB) and output (48 lines, 512 characters per line).
            // Keep diagnostic I/O failure secondary to the unchanged assertion.
            let trace_errors = (|| -> std::io::Result<()> {
                let file = fs::File::open(&trace)?;
                let mut lines = BufReader::new(file.take(64 * 1024 * 1024)).lines();
                let mut found = false;
                for line in lines.by_ref() {
                    let line = line?;
                    if line.trim_start().starts_with("node_errors:") {
                        eprintln!("recursive CHECK workers={workers} first producer errors:");
                        eprintln!("{}", line.chars().take(512).collect::<String>());
                        found = true;
                        break;
                    }
                }
                if found {
                    for line in lines.take(47) {
                        let line = line?;
                        if line.trim_start().starts_with("demands:") {
                            break;
                        }
                        eprintln!("{}", line.chars().take(512).collect::<String>());
                    }
                } else {
                    eprintln!(
                        "recursive CHECK workers={workers}: node_errors section absent in bounded trace scan"
                    );
                }
                Ok(())
            })();
            if let Err(error) = trace_errors {
                eprintln!("recursive CHECK trace diagnostic read failed: {error}");
            }
        }
        assert_eq!(
            result.status.code(),
            Some(0),
            "stock check accepts: {result:?}"
        );
    }
    rejects_resource_build(
        r#"fn grow<T>(x:T){grow::<(T,)>((x,));}fn run()->i32{grow::<i32>(1);7}
fn main(){if run()==7{println!("generic oracle ok");}}"#,
    );
}

#[test]
fn generic_body_move_error_is_attributed_to_generic_item() {
    // Stock rustc rejects with E0382 in `twice`, for every instantiation.
    let source = r#"fn twice<T>(x:T)->(T,T){let a=x;let b=x;(a,b)}
fn main(){let _=twice::<i32>(1);let _=twice::<String>(String::new());}"#;
    rejects("generic move error", source);
    for workers in [1, 8] {
        let scratch = Scratch::new();
        let input = scratch.source(source);
        let output = scratch.path("must-not-exist");
        let result = capture(&mut compiler(&input, &output, workers, true), &scratch, 20);
        let stderr = String::from_utf8_lossy(&result.stderr);
        assert!(
            stderr.contains("twice"),
            "not attributed to generic item: {stderr}"
        );
        assert!(
            !stderr.contains(&usize::MAX.to_string()),
            "sentinel identity leaked: {stderr}"
        );
    }
}

const CELL_VALID: &[(&str, &str)] = &[
    (
        "canonical_core_std",
        r#"fn main(){let x:core::cell::Cell<usize>=std::cell::Cell::new(1usize);core::cell::Cell::set(&x,7usize);if std::cell::Cell::get(&x)==7usize{println!("7");}}"#,
    ),
    (
        "imported_alias",
        r#"use core::cell::Cell as Renamed;fn main(){let x:Renamed<usize>=Renamed::new(1usize);x.set(7usize);if x.get()==7usize{println!("7");}}"#,
    ),
    (
        "shared_aliases",
        r#"use std::cell::Cell;fn main(){let x=Cell::new(1usize);let a=&x;let b=&x;a.set(7usize);if b.get()==7usize{println!("7");}}"#,
    ),
    (
        "shared_receiver",
        r#"use std::cell::Cell;fn bump(x:&Cell<usize>){x.set(x.get()+1usize);}fn main(){let x=Cell::new(6usize);bump(&x);if x.get()==7usize{println!("7");}}"#,
    ),
    (
        "closure_capture",
        r#"use std::cell::Cell;fn main(){let x=Cell::new(6usize);let f=||{x.set(x.get()+1usize);};f();if x.get()==7usize{println!("7");}}"#,
    ),
    (
        "field_storage",
        r#"use std::cell::Cell;struct Holder{x:Cell<usize>}fn main(){let h=Holder{x:Cell::new(1usize)};let a=&h.x;h.x.set(7usize);if a.get()==7usize{println!("7");}}"#,
    ),
    (
        "bool_char_float",
        r#"use std::cell::Cell;fn main(){let b=Cell::new(false);let c=Cell::new('a');let f=Cell::new(1.0f64);b.set(true);c.set('z');f.set(7.0f64);if b.get()&&c.get()=='z'&&f.get()==7.0f64{println!("7");}}"#,
    ),
    (
        "wide_scalars",
        r#"use std::cell::Cell;fn main(){let a=Cell::new(-1i128);let b=Cell::new(1u128);a.set(7i128);b.set(7u128);if a.get()==7i128&&b.get()==7u128{println!("7");}}"#,
    ),
    (
        "local_shadow",
        r#"struct Cell{value:i32}impl Cell{fn new(value:i32)->Self{Self{value}}fn get(&self)->i32{self.value}}fn main(){let x=Cell::new(7);if x.get()==7{println!("7");}}"#,
    ),
];
const CELL_INVALID: &[(&str, &str)] = &[
    (
        "not_copy",
        "use std::cell::Cell;fn main(){let x=Cell::new(1usize);let y=x;let a=x.get();let b=y.get();}",
    ),
    (
        "exclusive_conflict",
        "use std::cell::Cell;fn main(){let mut x=Cell::new(1usize);let a=&mut x;x.set(7usize);a.set(8usize);}",
    ),
    (
        "dangling_receiver",
        "use std::cell::Cell;fn main(){let a;{let x=Cell::new(1usize);a=&x;}let y=a.get();}",
    ),
    (
        "wrong_set_type",
        "use std::cell::Cell;fn main(){let x=Cell::new(1usize);x.set(true);}",
    ),
    (
        "wrong_get_arity",
        "use std::cell::Cell;fn main(){let x=Cell::new(1usize);x.get(2usize);}",
    ),
    ("unimported_type", "fn main(){let x=Cell::new(1usize);}"),
    (
        "missing_type_argument",
        "fn unused(x:core::cell::Cell){}fn main(){}",
    ),
];
const CELL_UNSUPPORTED: &[(&str, &str)] = &[
    (
        "generic_payload",
        r#"fn unused<T>(x:core::cell::Cell<T>){}fn main(){println!("7");}"#,
    ),
    (
        "reference_payload",
        r#"fn main(){let n=1;let x=core::cell::Cell::new(&n);println!("7");}"#,
    ),
    (
        "aggregate_payload",
        r#"fn main(){let x=core::cell::Cell::new((1,2));println!("7");}"#,
    ),
    (
        "unit_payload",
        r#"fn main(){let x=core::cell::Cell::new(());println!("7");}"#,
    ),
    (
        "get_mut",
        r#"fn main(){let mut x=core::cell::Cell::new(1usize);let a=x.get_mut();*a=7usize;println!("7");}"#,
    ),
    (
        "as_ptr",
        r#"fn main(){let x=core::cell::Cell::new(1usize);let p=x.as_ptr();println!("7");}"#,
    ),
    (
        "replace",
        r#"fn main(){let x=core::cell::Cell::new(1usize);let old=x.replace(7usize);println!("7");}"#,
    ),
    (
        "first_class_new",
        r#"fn main(){let f=core::cell::Cell::<usize>::new;let x=f(7usize);println!("7");}"#,
    ),
];
#[test]
fn cell_exact_stock_baseline() {
    let rustc = defaults_stock::stock_rustc();
    for (label, source) in CELL_VALID.iter().chain(CELL_UNSUPPORTED) {
        let scratch = Scratch::new();
        let input = scratch.source(source);
        for check in [true, false] {
            let output = scratch.path(if check {
                "stock.rmeta"
            } else {
                "stock-program"
            });
            let mut command = Command::new(&rustc);
            command
                .arg(&input)
                .args(["--edition=2024", "--crate-name=cell"]);
            if check {
                command.arg("--emit=metadata");
            }
            let result = capture(command.arg("-o").arg(&output), &scratch, 20);
            assert!(result.status.success(), "{label} check{check}: {result:?}");
            if !check {
                let result = capture(&mut Command::new(&output), &scratch, 5);
                assert!(result.status.success(), "{label}: {result:?}");
                assert_eq!(result.stdout, b"7\n", "{label}");
            }
        }
    }
    for (label, source) in CELL_INVALID {
        let scratch = Scratch::new();
        let input = scratch.source(source);
        for check in [true, false] {
            let output = scratch.path(if check {
                "invalid.rmeta"
            } else {
                "invalid-program"
            });
            let mut command = Command::new(&rustc);
            command
                .arg(&input)
                .args(["--edition=2024", "--crate-name=cell"]);
            if check {
                command.arg("--emit=metadata");
            }
            let result = capture(command.arg("-o").arg(&output), &scratch, 20);
            assert!(
                !result.status.success(),
                "{label}: stock accepted check{check}"
            );
            assert!(!output.exists(), "{label}: invalid stock output");
        }
    }
}

#[test]
fn cell_native_check_build_run_workers_1_8() {
    for (label, source) in CELL_VALID {
        eprintln!("Cell valid: {label}");
        runs(source, "7\n");
    }
    for (label, source) in CELL_INVALID {
        rejects(label, source);
    }
    for (label, source) in CELL_UNSUPPORTED {
        rejects(label, source);
        for workers in [1, 8] {
            let scratch = Scratch::new();
            let input = scratch.source(source);
            let output = scratch.path("unsupported-output");
            for check in [true, false] {
                let result = capture(&mut compiler(&input, &output, workers, check), &scratch, 20);
                let diagnostic = String::from_utf8_lossy(&result.stderr);
                assert!(diagnostic.contains("unsupported:"), "{label}: {diagnostic}");
                assert!(!output.exists(), "{label}: unsupported output");
            }
        }
    }
}
