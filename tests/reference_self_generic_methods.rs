//! Generic trait methods (own method binder) implemented for a reference-typed
//! Self (`impl<T: Tr> Tr for &T`) with SIZED ordinary actuals. The impl's
//! expected signature realizes per-layer reference topology from the item's
//! origin tails with the same kernel ordinary instantiation uses
//! (traits/generic_methods.rs). `?Sized` actuals (`T := str`) are tested below.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/closure_oracle.rs"]
mod oracle;
use oracle::{Case, Expect};

#[test]
fn sized_reference_self_forwarding() {
    oracle::check(
        "reference_self_generic_methods_sized_reference_self_forwarding",
        &[Case {
            name: "sized_reference_self_forwarding",
            source: "trait Sink { fn put(&mut self, n: usize); }\ntrait Feed { fn feed<S: Sink>(&self, s: &mut S); }\nimpl Feed for u32 { fn feed<S: Sink>(&self, s: &mut S) { s.put(*self as usize); } }\nimpl<T: Feed> Feed for &T { fn feed<S: Sink>(&self, s: &mut S) { s.put(1000); (**self).feed(s); } }\nstruct Log(Vec<usize>);\nimpl Sink for Log { fn put(&mut self, n: usize) { self.0.push(n); } }\nfn main() {\n    let mut l = Log(Vec::new());\n    let x = 7u32;\n    let r = &x;\n    r.feed(&mut l);\n    let rr: &&u32 = &r;\n    rr.feed(&mut l);\n    x.feed(&mut l);\n    println!(\"{:?}\", l.0);\n}",
            expect: Expect::Run("[7, 1000, 7, 7]\n"),
        }],
    );
}

#[test]
fn zero_layer_self_unchanged() {
    oracle::check(
        "reference_self_generic_methods_zero_layer_self_unchanged",
        &[Case {
            name: "zero_layer_self_unchanged",
            source: "trait Sink { fn put(&mut self, n: usize); }\ntrait Feed { fn feed<S: Sink>(&self, s: &mut S); }\nstruct V(usize);\nimpl Feed for V { fn feed<S: Sink>(&self, s: &mut S) { s.put(self.0); } }\nstruct Log(Vec<usize>);\nimpl Sink for Log { fn put(&mut self, n: usize) { self.0.push(n); } }\nfn go<F: Feed>(f: &F, l: &mut Log) { f.feed(l); }\nfn main() { let mut l = Log(Vec::new()); V(2).feed(&mut l); go(&V(3), &mut l); println!(\"{:?}\", l.0); }",
            expect: Expect::Run("[2, 3]\n"),
        }],
    );
}

/// OBSERVED REJECTED DEBT (not accepted compatibility, not a baseline): the
/// borrow-expression receiver `(&r).feed(&x)` is rejected natively with
/// "unsupported unresolved receiver type" (`method_receiver_hint` resolves a
/// borrow receiver only for arrays). The typed-local variant below covers the
/// reference method actual itself.
#[test]
fn reference_method_actual_borrow_receiver_is_rejected_debt() {
    let source = "trait Feed { fn feed<S>(&self, s: S) -> usize; }\nstruct V;\nimpl Feed for V { fn feed<S>(&self, _s: S) -> usize { 1 } }\nimpl<T: Feed> Feed for &T { fn feed<S>(&self, s: S) -> usize { (**self).feed(s) + 1 } }\nfn main() { let v = V; let r = &v; let x = 5u8; println!(\"{}\", (&r).feed(&x)); }";
    let stock = std::env::var_os("JUSTRUST_REAL_RUSTC")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("rustc"));
    let dir = std::env::temp_dir().join(format!("jr-ref-self-borrow-recv-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("main.rs");
    std::fs::write(&file, source).unwrap();
    let status = std::process::Command::new(&stock)
        .args(["--edition", "2024", "-o"])
        .arg(dir.join("stock"))
        .arg(&file)
        .status()
        .unwrap();
    assert!(
        status.success(),
        "stock must accept the borrow-expression receiver"
    );
    for workers in [1, 8] {
        for check_only in [true, false] {
            let error = justrust_engine::compiler::compile(source.into(), workers, check_only)
                .err()
                .unwrap_or_else(|| panic!("workers={workers}: native unexpectedly accepted"));
            assert!(
                error
                    .to_string()
                    .contains("unsupported unresolved receiver type"),
                "workers={workers} check={check_only}: unexpected rejection: {error}"
            );
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn reference_method_actual_typed_receiver() {
    oracle::check(
        "reference_self_generic_methods_reference_method_actual_typed_receiver",
        &[Case {
            name: "reference_method_actual_typed_receiver",
            source: "trait Feed { fn feed<S>(&self, s: S) -> usize; }\nstruct V;\nimpl Feed for V { fn feed<S>(&self, _s: S) -> usize { 1 } }\nimpl<T: Feed> Feed for &T { fn feed<S>(&self, s: S) -> usize { (**self).feed(s) + 1 } }\nfn main() { let v = V; let r = &v; let rr = &r; let x = 5u8; println!(\"{}\", rr.feed(&x)); }",
            expect: Expect::Run("2\n"),
        }],
    );
}

/// Unsized `T := str` behind `impl<T: ?Sized + Feed> Feed for &T`. The
/// instance layout of `&T`/`&&T` with `T := str` is the two-word (data, len)
/// carrier through loads, `&**self` reborrows, calls and returns.
#[test]
fn unsized_str_generic_actual_keeps_fat_metadata() {
    oracle::check(
        "reference_self_generic_methods_unsized_str_generic_actual",
        &[
            Case {
                name: "slice_free_variant",
                source: "trait Sink {\n    fn put(&mut self, n: usize);\n}\ntrait Feed {\n    fn feed<S: Sink>(&self, s: &mut S);\n}\nimpl Feed for str {\n    fn feed<S: Sink>(&self, s: &mut S) {\n        s.put(self.len());\n        s.put(255);\n    }\n}\nimpl<T: ?Sized + Feed> Feed for &T {\n    fn feed<S: Sink>(&self, s: &mut S) {\n        (**self).feed(s);\n    }\n}\nstruct Log {\n    n: Vec<usize>,\n}\nimpl Sink for Log {\n    fn put(&mut self, n: usize) {\n        self.n.push(n);\n    }\n}\nfn main() {\n    let mut l = Log { n: Vec::new() };\n    \"ab\".feed(&mut l);\n    let r: &&str = &\"xyz\";\n    r.feed(&mut l);\n    println!(\"{:?}\", l.n);\n}",
                expect: Expect::Run("[2, 255, 3, 255]\n"),
            },
            Case {
                name: "deref_deref_len",
                source: "trait L { fn l(&self) -> usize; }\nimpl L for str { fn l(&self) -> usize { self.len() } }\nimpl<T: ?Sized + L> L for &T { fn l(&self) -> usize { (**self).l() } }\nfn f<T: ?Sized + L>(x: &T) -> usize { x.l() }\nfn g<T: ?Sized + L>(x: &&T) -> usize { (**x).l() }\nfn h<T: ?Sized + L>(x: &&T) -> usize { let y: &T = *x; y.l() }\nfn main() {\n    let x: &str = \"abc\";\n    let r: &&str = &x;\n    println!(\"{} {} {}\", x.l(), r.l(), (**r).l());\n    println!(\"{} {} {}\", f(x), g(&x), h(&x));\n}",
                expect: Expect::Run("3 3 3\n3 3 3\n"),
            },
            Case {
                name: "sized_and_unsized_actuals_with_returns",
                source: "trait L { fn l(&self) -> usize; }\nimpl L for str { fn l(&self) -> usize { self.len() } }\nimpl L for u32 { fn l(&self) -> usize { *self as usize } }\nimpl<T: ?Sized + L> L for &T { fn l(&self) -> usize { 100 + (**self).l() } }\nfn id<T: ?Sized>(x: &T) -> &T { x }\nfn ff<T: ?Sized + L>(x: &&T) -> usize { let y: &T = id(*x); y.l() + x.l() }\nfn main() {\n    let x: &str = \"abcd\"; let n = 9u32; let rn = &n;\n    println!(\"{} {} {}\", ff(&x), ff(&rn), id(x).len());\n    let s: &str = id::<str>(\"hello\"); println!(\"{}\", s);\n}",
                expect: Expect::Run("108 118 4\nhello\n"),
            },
            Case {
                name: "implicit_sized_rejects_str_function_actual",
                source: "fn f<T>(_x: &T) -> usize { 0 }\nfn main() { println!(\"{}\", f::<str>(\"ab\")); }",
                expect: Expect::Invalid("E0277"),
            },
            Case {
                name: "implicit_sized_rejects_inferred_str_actual",
                source: "trait L { fn l(&self) -> usize; }\nimpl L for str { fn l(&self) -> usize { self.len() } }\nfn g<T: L>(x: &&T) -> usize { (**x).l() }\nfn main() { let x: &str = \"ab\"; println!(\"{}\", g(&x)); }",
                expect: Expect::Invalid("E0277"),
            },
        ],
    );
}

/// The original probe still stops at the independent `str::as_bytes` gap
/// (not at the unsized generic actual). Boundary, not acceptance.
#[test]
fn unsized_str_probe_as_bytes_is_an_explicit_boundary() {
    let source = "trait Sink {\n    fn put(&mut self, bytes: &[u8]);\n}\ntrait Feed {\n    fn feed<S: Sink>(&self, s: &mut S);\n}\nimpl Feed for str {\n    fn feed<S: Sink>(&self, s: &mut S) {\n        s.put(self.as_bytes());\n        s.put(&[255u8]);\n    }\n}\nimpl<T: ?Sized + Feed> Feed for &T {\n    fn feed<S: Sink>(&self, s: &mut S) {\n        (**self).feed(s);\n    }\n}\nstruct Log {\n    n: Vec<usize>,\n}\nimpl Sink for Log {\n    fn put(&mut self, bytes: &[u8]) {\n        self.n.push(bytes.len());\n    }\n}\nfn main() {\n    let mut l = Log { n: Vec::new() };\n    \"ab\".feed(&mut l);\n    let r: &&str = &\"xyz\";\n    r.feed(&mut l);\n    println!(\"{:?}\", l.n);\n}";
    for workers in [1, 8] {
        for check_only in [true, false] {
            let error = justrust_engine::compiler::compile(source.into(), workers, check_only)
                .err()
                .unwrap_or_else(|| panic!("workers={workers}: native must reject"));
            assert!(
                error.to_string().contains("no method named `as_bytes`"),
                "workers={workers} check={check_only}: unexpected boundary: {error}"
            );
        }
    }
}

/// OUT OF SCOPE, observed REJECTED (not accepted, not a baseline or
/// regression proof). A method returning `&Self` in `impl<T> Tr for &T`: the
/// impl's own source contract (`canonical_ref_contract`, frontend/type_check.rs)
/// expands the reference Self layers for the receiver input but gives the
/// `&Self` result only its one explicit layer, while `&Self = &&T` has two.
/// The validator then rejects it. Debt: nested Self result-layer elision in
/// the source contract kernel; not changed in this slice. Both programs are
/// valid for stock Rust (the second is a stock E0515 program, so its native
/// rejection here says nothing about lifetime soundness).
#[test]
fn reference_self_returning_self_is_rejected_out_of_scope() {
    let stock = std::env::var_os("JUSTRUST_REAL_RUSTC")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("rustc"));
    for (name, source, stock_ok) in [
        (
            "forward_and_return_self",
            "trait Sink { fn put(&mut self, n: usize); }\ntrait Feed { fn feed<S: Sink>(&self, s: &mut S); fn pick<S: Sink>(&self, s: &mut S) -> &Self; }\nstruct V(usize);\nimpl Feed for V {\n    fn feed<S: Sink>(&self, s: &mut S) { s.put(self.0); }\n    fn pick<S: Sink>(&self, s: &mut S) -> &Self { s.put(self.0 + 100); self }\n}\nimpl<T: Feed> Feed for &T {\n    fn feed<S: Sink>(&self, s: &mut S) { (**self).feed(s); }\n    fn pick<S: Sink>(&self, s: &mut S) -> &Self { (**self).pick(s); self }\n}\nstruct Log(Vec<usize>);\nimpl Sink for Log { fn put(&mut self, n: usize) { self.0.push(n); } }\nfn main() {\n    let mut l = Log(Vec::new());\n    let v = V(7);\n    let r = &v;\n    r.feed(&mut l);\n    (&r).feed::<Log>(&mut l);\n    let back: &&V = (&r).pick(&mut l);\n    back.feed(&mut l);\n    V(1).feed(&mut l);\n    println!(\"{:?}\", l.0);\n}",
            true,
        ),
        (
            "returned_self_cannot_escape_local",
            "trait Feed { fn pick<S>(&self, s: S) -> &Self; }\nstruct V;\nimpl Feed for V { fn pick<S>(&self, _s: S) -> &Self { self } }\nimpl<T: Feed> Feed for &T { fn pick<S>(&self, _s: S) -> &Self { self } }\nfn bad<'x>(v: &'x V) -> &'x &'x V { let r: &V = v; (&r).pick(0u8) }\nfn main() {}",
            false,
        ),
    ] {
        let dir =
            std::env::temp_dir().join(format!("jr-ref-self-return-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("main.rs");
        std::fs::write(&file, source).unwrap();
        let status = std::process::Command::new(&stock)
            .args(["--edition", "2024", "-o"])
            .arg(dir.join("stock"))
            .arg(&file)
            .status()
            .unwrap();
        assert_eq!(status.success(), stock_ok, "{name}: stock outcome changed");
        for workers in [1, 8] {
            for check_only in [true, false] {
                let error = justrust_engine::compiler::compile(source.into(), workers, check_only)
                    .err()
                    .unwrap_or_else(|| panic!("{name} workers={workers}: native must reject"));
                assert!(
                    error
                        .to_string()
                        .contains("invalid noescape reference contract: reference result position coverage mismatch"),
                    "{name} workers={workers} check={check_only}: unexpected rejection: {error}"
                );
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
