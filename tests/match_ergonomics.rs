//! Match ergonomics: enum matches through dereferenced places, RFC 2005
//! default binding modes (edition 2024 rules), `ref`/`ref mut` bindings and
//! explicit `&pat`/`&mut pat`, measured against stock rustc. See MATCH-ERGONOMICS.md.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/oracle.rs"]
mod oracle;
use oracle::{Case, Expect};

fn cases() -> Vec<Case> {
    vec![
        Case {
            name: "deref_match_payloads",
            source: "enum E { A(i32), B(i32), C }\nfn get(e: &E) -> i32 { match *e { E::A(x) => x, E::B(y) => y + 1, E::C => 0 } }\nfn main() { if {let a=E::B(41);let b=E::A(7);let c=E::C; get(&a) == 42 && get(&b) == 7 && get(&c) == 0} { println!(\"ok\") } }",
            expect: Expect::Run("ok\n"),
        },
        Case {
            name: "deref_match_mut_reassign",
            source: "enum E { A(i32), B }\nfn bump(e: &mut E) { match *e { E::A(x) => { *e = E::A(x + 1); } E::B => {} } }\nfn main() { let mut e = E::A(41); bump(&mut e); if let E::A(v) = e { if v == 42 { println!(\"ok\") } } }",
            expect: Expect::Run("ok\n"),
        },
        Case {
            name: "deref_if_let_let_else",
            source: "enum E { A(i32), B }\nfn f(e: &E) -> i32 { if let E::A(x) = *e { x } else { 0 } }\nfn g(e: &E) -> i32 { let E::A(x) = *e else { return 5 }; x }\nfn main() { if {let a=E::A(2);let b=E::B; f(&a) + g(&b)} == 7 { println!(\"ok\") } }",
            expect: Expect::Run("ok\n"),
        },
        Case {
            name: "deref_switch_unit_payload",
            source: "enum E { A(i32), B(i32) }\nfn get(e: &E) -> i32 { match *e { E::A(_) => 1, E::B(_) => 2 } }\nfn main() { let e = E::B(41); let r = &e; if get(r) == 2 { println!(\"ok\") } }",
            expect: Expect::Run("ok\n"),
        },
        Case {
            name: "deref_projected_field_match",
            source: "enum E { A(i32), B }\nstruct H { e: E }\nfn f(h: &H) -> i32 { match (*h).e { E::A(x) => x, E::B => 9 } }\nfn main() { if {let h=H { e: E::B }; f(&h)} == 9 { println!(\"ok\") } }",
            expect: Expect::Run("ok\n"),
        },
        Case {
            name: "deref_mut_assign_in_arm",
            source: "enum E { A(i32), B }\nfn main() { let mut a = E::A(4); let r = &mut a;\n  let v = match *r { E::A(x) => { *r = E::B; x } E::B => 0 }; if v == 4 { println!(\"ok\") } }",
            expect: Expect::Run("ok\n"),
        },
        Case {
            name: "self_method_default_binding",
            source: "enum E { A(i32), B(i32) }\nimpl E { fn get(&self) -> i32 { match self { E::A(r) => *r, E::B(r) => *r + 1 } } }\nfn main() { let e = E::B(41); if e.get() == 42 { println!(\"ok\") } }",
            expect: Expect::Run("ok\n"),
        },
        Case {
            name: "struct_let_default_binding",
            source: "struct P { x: i32, y: i32 }\nfn sum(p: &P) -> i32 { let P { x, y } = p; *x + *y }\nfn main() { let p = P { x: 40, y: 2 }; if sum(&p) == 42 { println!(\"ok\") } }",
            expect: Expect::Run("ok\n"),
        },
        Case {
            name: "noncopy_payload_by_reference",
            source: "struct S { v: i32 }\nenum E { A(S), B }\nfn get(e: &E) -> i32 { match e { E::A(s) => s.v, E::B => 0 } }\nfn main() { let a = E::A(S { v: 3 }); let b = E::B; if get(&a) + get(&b) == 3 { println!(\"ok\") } }",
            expect: Expect::Run("ok\n"),
        },
        Case {
            name: "ref_and_ref_mut_bindings",
            source: "enum E { A(i32), B }\nfn main() { let mut e = E::A(1); match e { E::A(ref mut x) => { *x += 41; } E::B => {} }\n  match e { E::A(ref x) => { if *x == 42 { println!(\"ok\") } } E::B => {} } }",
            expect: Expect::Run("ok\n"),
        },
        Case {
            name: "explicit_ref_pattern_and_default",
            source: "fn main() { let p = (1, 2); let r = &p; let &(a, b) = r; let (c, d) = r; if a + b + *c + *d == 6 { println!(\"ok\") } }",
            expect: Expect::Run("ok\n"),
        },
        Case {
            name: "if_let_on_borrow",
            source: "enum E { A(i32), B }\nfn main() { let e = E::A(5); if let E::A(x) = &e { if *x == 5 { println!(\"ok\") } } }",
            expect: Expect::Run("ok\n"),
        },
        Case {
            name: "nested_struct_enum_ergonomics",
            source: "struct In { a: i32 }\nstruct Out { i: In, e: Option2 }\nenum Option2 { S(In), N }\nfn f(o: &Out) -> i32 { match o { Out { i: In { a }, e: Option2::S(In { a: b }) } => *a + *b, Out { i, e: Option2::N } => i.a } }\nfn main() { let o = Out { i: In { a: 1 }, e: Option2::S(In { a: 2 }) }; let p = Out { i: In { a: 7 }, e: Option2::N }; if f(&o) + f(&p) == 10 { println!(\"ok\") } }",
            expect: Expect::Run("ok\n"),
        },
        Case {
            name: "match_mut_borrow_then_read",
            source: "enum E { A(i32), B }\nfn main() { let mut e = E::A(1); match &mut e { E::A(x) => { *x = 2; } E::B => {} }; if let E::A(2) = e { println!(\"ok\") } }",
            expect: Expect::Run("ok\n"),
        },
        Case {
            name: "tuple_mut_default_binding",
            source: "struct P { x: i32 }\nfn main() { let mut p = (P { x: 1 }, 2); { let (a, b) = &mut p; a.x += *b; } if p.0.x == 3 { println!(\"ok\") } }",
            expect: Expect::Run("ok\n"),
        },
        Case {
            name: "nested_tuple_mixed_ref_patterns",
            source: "fn main() { let a = (1, (2, 3)); let r = &a; let (x, (y, z)) = r; let &(_, (ref w, _)) = r; if *x + *y + *z + *w == 8 { println!(\"ok\") } }",
            expect: Expect::Run("ok\n"),
        },
        Case {
            name: "literal_tests_through_reference",
            source: "enum E { A(i32), B(bool) }\nfn f(e: &E) -> i32 { match e { E::A(0) => 10, E::A(n) => *n, E::B(true) => 1, E::B(false) => 2 } }\nfn main() { let a = E::A(0); let b = E::A(4); let c = E::B(false); if f(&a) + f(&b) + f(&c) == 16 { println!(\"ok\") } }",
            expect: Expect::Run("ok\n"),
        },
        Case {
            name: "return_default_binding",
            source: "fn g(p: &(i32, i32)) -> &i32 { let (a, _) = p; a }\nfn main() { let p = (5, 6); if *g(&p) == 5 { println!(\"ok\") } }",
            expect: Expect::Run("ok\n"),
        },
        Case {
            name: "return_ref_binding",
            source: "fn g(p: &(i32, i32)) -> &i32 { let &(ref a, _) = p; a }\nfn main() { let p = (5, 6); if *g(&p) == 5 { println!(\"ok\") } }",
            expect: Expect::Run("ok\n"),
        },
        Case {
            name: "move_payload_out_of_deref",
            source: "struct S { v: i32 }\nenum E { A(S), B }\nfn get(e: &E) -> i32 { match *e { E::A(s) => s.v, E::B => 0 } }\nfn main() { if {let a=E::A(S { v: 3 }); get(&a)} == 3 { println!(\"ok\") } }",
            expect: Expect::Invalid("E0507"),
        },
        Case {
            name: "nonexhaustive_through_deref",
            source: "enum E { A(i32), B }\nfn f(e: &E) -> i32 { match *e { E::A(x) => x } }\nfn main() { if {let a=E::A(2); f(&a) == 2} { println!(\"ok\") } }",
            expect: Expect::Invalid("E0004"),
        },
        Case {
            name: "mut_binding_conflicts_shared",
            source: "enum E { A(i32), B }\nfn main() { let mut e = E::A(1); let r = match &mut e { E::A(x) => x, E::B => return }; let s = &e; *r += 1; let _ = s; }",
            expect: Expect::Invalid("E0502"),
        },
        Case {
            name: "assign_while_ref_mut_live",
            source: "enum E { A(i32), B }\nfn main() { let mut e = E::A(1); if let E::A(ref mut x) = e { e = E::B; *x += 1; } }",
            expect: Expect::Invalid("E0506"),
        },
        Case {
            name: "ref_mut_of_immutable",
            source: "enum E { A(i32), B }\nfn main() { let e = E::A(1); if let E::A(ref mut x) = e { *x += 1; } }",
            expect: Expect::Invalid("E0596"),
        },
        Case {
            name: "move_out_of_default_binding",
            source: "struct S { v: i32 }\nenum E { A(S), B }\nfn take(e: &E) -> S { match e { E::A(s) => *s, E::B => S { v: 0 } } }\nfn main() { }",
            expect: Expect::Invalid("E0507"),
        },
        Case {
            name: "move_through_explicit_ref_pattern",
            source: "struct S { v: i32 }\nfn main() { let s = (S { v: 1 },); let r = &s; let &(t,) = r; }",
            expect: Expect::Invalid("E0507"),
        },
        Case {
            name: "mut_match_while_shared_live",
            source: "enum E { A(i32), B }\nfn main() { let mut e = E::A(1); let r = &e; match &mut e { E::A(x) => { *x += 1; } E::B => {} }; if let E::A(v) = r { let _ = v; } }",
            expect: Expect::Invalid("E0502"),
        },
        Case {
            name: "assign_through_shared_default_binding",
            source: "enum E { A(i32), B }\nfn main() { let e = E::A(1); match &e { E::A(x) => { *x += 1; } E::B => {} }; }",
            expect: Expect::Invalid("E0594"),
        },
        Case {
            name: "mut_binding_in_implicit_borrow",
            source: "fn main() { let p = (1, 2); let r = &p; let (mut a, b) = r; }",
            expect: Expect::Invalid("cannot mutably bind by value"),
        },
        Case {
            name: "ref_binding_in_implicit_borrow",
            source: "fn main() { let p = (1, 2); let r = &p; let (ref a, b) = r; }",
            expect: Expect::Invalid("cannot explicitly borrow"),
        },
    ]
}

#[test]
fn match_ergonomics_agree_with_stock_rustc() {
    oracle::check("match-ergonomics", &cases());
}
