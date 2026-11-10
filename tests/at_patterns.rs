//! `name @ pattern` bindings, checked against stock rustc.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/oracle.rs"]
mod oracle;
use oracle::{Case, Expect};
#[path = "support/closure_oracle.rs"]
mod closure_oracle;

#[test]
fn at_patterns_match_stock() {
    oracle::check(
        "at-patterns",
        &[
            Case {
                name: "scalar_ranges",
                source: r#"fn f(x: i32) -> i32 { match x { n @ 1..=5 => n * 10, n => n } }
fn g(c: char) -> i32 { match c { d @ '0'..='9' => d as i32 - 48, _ => -1 } }
fn main() { println!("{} {} {} {}", f(3), f(9), g('7'), g('x')); }"#,
                expect: Expect::Run("30 9 7 -1\n"),
            },
            Case {
                name: "nested_and_whole",
                source: r#"enum T { A(i32), B }
fn f(t: &T) -> i32 { match t { whole @ T::A(n @ 1..=3) => { let _ = whole; *n } T::A(_) => 0, T::B => -1 } }
fn main() {
    println!("{} {} {}", f(&T::A(2)), f(&T::A(7)), f(&T::B));
    let p = (1, 2);
    match p { all @ (a, _) => println!("{} {}", all.1, a) }
    let v = Some(4);
    if let Some(x @ (1 | 4)) = v { println!("{}", x); }
    match v { s @ Some(_) => println!("{}", s.is_some()), None => {} }
}"#,
                expect: Expect::Run("2 0 -1\n2 1\n4\ntrue\n"),
            },
            Case {
                name: "ref_at",
                source: r#"struct S { v: i32 }
fn main() { let s = S { v: 3 }; match s { ref w @ S { v } => println!("{} {}", w.v, v) } }"#,
                expect: Expect::Run("3 3\n"),
            },
            Case {
                name: "duplicate_binding",
                source: r#"fn main() { match 1 { x @ x => println!("{}", x) } }"#,
                expect: Expect::Invalid("E0416"),
            },
        ],
    );
}

/// The @ name's type comes only from local inference here: no annotation, a
/// literal scrutinee, and the whole value is used. Strict oracle: CHECK and
/// BUILD at workers 1 and 8, and CHECK must not demand code generation.
#[test]
fn inferred_at_bindings_match_stock_at_workers_1_and_8() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "at-inferred",
        &[
            Case {
                name: "inferred_whole_binding",
                source: r#"fn main() {
    let whole @ (a, b) = (3, 4);
    println!("{} {}", whole.0 + whole.1, a + b);
    let x = Some(42);
    match x { w @ Some(_) => match w { Some(n) => println!("{}", n), None => {} }, None => {} }
    let pair = (1u8, 2u8);
    if let all @ (1, _) = pair { println!("{}", all.1); }
}"#,
                expect: Expect::Run("7 7\n42\n2\n"),
            },
            Case {
                name: "inferred_whole_binding_type_mismatch",
                source: r#"fn main() { let whole @ (a, _) = (3, true); let _: bool = whole.0; let _ = a; }"#,
                expect: Expect::Invalid("E0308"),
            },
            // Unannotated pattern lets take literal types from the solved
            // initializer, like plain lets (`let _ = match .. { .. => y, _ => 0 }`).
            Case {
                name: "pattern_let_literal_arms_follow_initializer",
                source: r#"fn main() {
    let y = 1u8;
    let _ = match 1u8 { 1 => y, _ => 0 };
    let _ = if true { y } else { 0 };
    let (a, b) = (if false { y } else { 255 }, 2);
    println!("{} {}", a, b);
}"#,
                expect: Expect::Run("255 2\n"),
            },
            Case {
                name: "pattern_let_literal_arm_overflow",
                source: r#"fn main() { let y = 1u8; let _ = if true { y } else { 300 }; }"#,
                expect: Expect::Invalid("literal out of range"),
            },
            // Subslice bindings `name @ ..` in slice patterns.
            Case {
                name: "subslice_recursive_sum",
                source: r#"fn sum(v: &[i32]) -> i32 {
    match v {
        [] => 0,
        [first, rest @ ..] => *first + sum(rest),
    }
}
fn main() { println!("{}", sum(&[1, 2, 3])); }"#,
                expect: Expect::Run("6\n"),
            },
            Case {
                name: "subslice_middle_and_lengths",
                source: r#"fn describe(v: &[i32]) {
    match v {
        [] => println!("empty"),
        [one] => println!("one {one}"),
        [a, b] => println!("two {a} {b}"),
        [first, mid @ .., last] => println!("{first} {mid:?} {last}"),
    }
}
fn main() { let v = [1, 2, 3, 4, 5]; for n in 0..6 { describe(&v[..n]); } }"#,
                expect: Expect::Run("empty\none 1\ntwo 1 2\n1 [2] 3\n1 [2, 3] 4\n1 [2, 3, 4] 5\n"),
            },
            Case {
                name: "subslice_guards_and_literals",
                source: r#"fn ends(v: &[u8]) -> u8 {
    match v {
        [0, ..] => 100,
        [.., 9] => 9,
        [head @ .., tail] if head.len() > 2 => *tail + head.len() as u8,
        [x, _, ..] if *x > 5 => 50,
        other => other.len() as u8,
    }
}
fn main() { println!("{} {} {} {} {}", ends(&[0, 1]), ends(&[1, 9]), ends(&[1, 2, 3, 4]), ends(&[7, 1]), ends(&[1])); }"#,
                expect: Expect::Run("100 9 7 50 1\n"),
            },
            Case {
                name: "subslice_through_mut_slice",
                source: r#"fn bump(v: &mut [i32]) {
    if let [first, ..] = v { *first += 10; }
    if let [.., last] = v { *last += 100; }
    if let [_, rest @ ..] = v { if let [x, ..] = rest { *x *= 2; } }
    match v { [0, ..] => println!("zero"), [_, mid, ..] => *mid += 1, _ => {} }
}
fn main() { let mut m = [1, 2, 3]; bump(&mut m); println!("{m:?}"); let mut e: [i32; 0] = []; bump(&mut e); }"#,
                expect: Expect::Run("[11, 5, 103]\n"),
            },
            Case {
                name: "subslice_loops_arrays_and_returns",
                source: r#"fn first_word(words: &[String]) -> &str { match words { [w, ..] => w.as_str(), [] => "" } }
fn tail<'a>(v: &'a [i32]) -> &'a [i32] { match v { [_, rest @ ..] => rest, [] => v } }
fn main() {
    let v = vec![1, 2, 3, 4, 5];
    let words = vec![String::from("a"), String::from("bb"), String::from("ccc")];
    if let [w, others @ ..] = words.as_slice() { println!("{w} {}", others.len()); }
    println!("{} {:?}", first_word(&words), tail(&v));
    let mut it = 0;
    while let [_, rest @ ..] = &v[it..] { it += 1; if rest.is_empty() { break; } }
    println!("{it}");
    let r = match &v[..] { [x, ..] if *x > 3 => "big", [_, _, ..] => "pair", _ => "small" };
    println!("{r}");
    let nested: &[(i32, bool)] = &[(1, true), (2, false)];
    match nested { [(n, true), ..] => println!("t {n}"), [(n, false), ..] => println!("f {n}"), _ => println!("none") }
    let mut total = 0;
    for chunk in [&v[..2], &v[2..], &v[5..]] {
        total += match chunk { [a, b] => a * b, [a, .., c] => a + c, [a] => *a, [] => -1 };
    }
    println!("{total}");
    'outer: for i in 0..3 {
        match &v[i..] { [x, ..] if *x == 2 => continue 'outer, [x, ..] => { if *x == 3 { break; } println!("x {x}"); } [] => {} }
    }
    let arr = [7u8, 8, 9];
    let [a, .., c] = arr;
    if let [h, t @ ..] = &arr { println!("{a} {c} {h} {}", t.len()); }
}"#,
                expect: Expect::Run("a 2\na [2, 3, 4, 5]\n5\npair\nt 1\n9\nx 1\n7 9 7 2\n"),
            },
            Case {
                name: "subslice_non_exhaustive_lengths",
                source: r#"fn main() { let v = [1, 2, 3]; match &v[..] { [a] => println!("{a}"), [a, b] => println!("{a}{b}") } }"#,
                expect: Expect::Invalid("E0004"),
            },
            Case {
                name: "subslice_two_rests",
                source: r#"fn main() { let v = [1, 2, 3]; if let [a @ .., b @ ..] = &v[..] { println!("{}", a.len() + b.len()); } }"#,
                expect: Expect::Invalid("can only be used once per slice pattern"),
            },
            Case {
                name: "subslice_binding_is_a_slice",
                source: r#"fn main() { let v = [1, 2, 3]; if let [a, rest @ ..] = &v[..] { let n: i32 = rest; println!("{a} {n}"); } }"#,
                expect: Expect::Invalid("E0308"),
            },
        ],
    );
}
