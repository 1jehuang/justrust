//! General or-patterns (bindings, nominal rows, nested, guarded), stock-checked.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/oracle.rs"]
mod oracle;
use oracle::{Case, Expect};

#[test]
fn or_patterns_match_stock() {
    oracle::check(
        "or-patterns",
        &[
            Case {
                name: "binding_rows",
                source: r#"enum T { A(i32), B(i32), C }
fn f(t: T) -> i32 { match t { T::A(x) | T::B(x) => x, T::C => 0 } }
fn main() { println!("{} {} {}", f(T::A(3)), f(T::B(4)), f(T::C)); }"#,
                expect: Expect::Run("3 4 0\n"),
            },
            Case {
                name: "enum_by_reference",
                source: r#"enum T { A, B, C }
fn f(t: &T) -> i32 { match t { T::A | T::B => 1, T::C => 2 } }
fn main() { println!("{} {}", f(&T::B), f(&T::C)); }"#,
                expect: Expect::Run("1 2\n"),
            },
            Case {
                name: "chars_and_ranges",
                source: r#"fn f(c: char) -> i32 { match c { '(' | '[' | '{' => 1, '0'..='9' | '.' => 2, _ => 0 } }
fn main() { println!("{} {} {} {}", f('['), f('.'), f('7'), f('x')); }"#,
                expect: Expect::Run("1 2 2 0\n"),
            },
            Case {
                name: "nested",
                source: r#"enum T { A(i32), B(i32), C }
fn g(o: Option<i32>) -> i32 { match o { Some(1 | 2) => 1, Some(_) => 2, None => 3 } }
fn f(t: (T, bool)) -> i32 { match t { (T::A(x) | T::B(x), true) => x, _ => 0 } }
fn main() {
    println!("{} {} {}", g(Some(2)), g(Some(5)), g(None));
    println!("{} {}", f((T::B(4), true)), f((T::C, true)));
    if let Some(1 | 3) = Some(3) { println!("y"); }
}"#,
                expect: Expect::Run("1 2 3\n4 0\ny\n"),
            },
            Case {
                name: "guarded",
                source: r#"fn f(x: i32) -> i32 { match x { 1 | 2 if x > 1 => 1, _ => 0 } }
fn main() { println!("{} {} {}", f(1), f(2), f(3)); }"#,
                expect: Expect::Run("0 1 0\n"),
            },
            Case {
                name: "unbound_in_one_row",
                source: r#"enum T { A(i32), B(i32) }
fn f(t: T) -> i32 { match t { T::A(x) | T::B(y) => x } }
fn main() { let _ = f(T::A(1)); }"#,
                expect: Expect::Invalid("E0408"),
            },
            Case {
                name: "fixture_bracket_pairs",
                source: r#"fn ok(open: char, c: char) -> bool { matches!((open, c), ('(', ')') | ('[', ']') | ('{', '}')) }
fn kind(c: char) -> i32 { match c { '(' | '[' | '{' => 1, ')' | ']' | '}' => 2, _ => 0 } }
fn main() { println!("{} {} {} {}", ok('[', ']'), ok('(', ']'), kind('{'), kind('}')); }"#,
                expect: Expect::Run("true false 1 2\n"),
            },
            Case {
                name: "nested_option_tuple",
                source: r#"fn f(p: (Option<i32>, i32)) -> i32 { match p { (Some(1 | 2), y) | (None, y) => y, (Some(_), _) => -1 } }
fn main() { println!("{} {} {}", f((Some(2), 5)), f((None, 6)), f((Some(9), 7))); }"#,
                expect: Expect::Run("5 6 -1\n"),
            },
            Case {
                name: "binding_type_mismatch",
                source: r#"enum T { A(i32), B(bool) }
fn f(t: T) -> i32 { match t { T::A(x) | T::B(x) => 1 } }
fn main() { let _ = f(T::A(1)); }"#,
                expect: Expect::Invalid("E0308"),
            },
            Case {
                name: "unbound_in_unused_item",
                source: r#"enum T { A(i32), B(i32) }
fn unused(t: T) -> i32 { match t { T::A(x) | T::B(y) => y } }
fn main() { println!("live"); }"#,
                expect: Expect::Invalid("E0408"),
            },
            Case {
                name: "inconsistent_ref_binding_unused",
                source: r#"fn unused(x: &Option<(i32, i32)>) -> i32 { match x { Some((x, 3)) | &Some((ref x, 5)) => *x, _ => 0 } }
fn main() { println!("live"); }"#,
                expect: Expect::Invalid("E0409"),
            },
            Case {
                name: "undeclared_struct_lifetime_unused",
                source: r#"struct S { a: &'a isize }
enum E { V(Option<&'b u8>) }
fn main() { println!("live"); }"#,
                expect: Expect::Invalid("E0261"),
            },
            Case {
                name: "private_assoc_type_in_public_impl",
                source: r#"struct Bar;
pub trait PubTr { type A; }
impl PubTr for u8 { type A = Bar; }
fn main() { println!("live"); }"#,
                expect: Expect::Invalid("E0446"),
            },
            Case {
                name: "private_assoc_type_for_private_self",
                source: r#"struct Bar;
struct Priv;
pub trait PubTr { type A; fn n() -> i32 { 7 } }
impl PubTr for Priv { type A = Bar; }
pub struct P;
impl PubTr for P { type A = P; }
fn main() { println!("{} {}", <Priv as PubTr>::n(), <P as PubTr>::n()); }"#,
                expect: Expect::Run("7 7\n"),
            },
            Case {
                name: "mut_ref_result_invariant",
                source: r#"struct S<'b>(&'b i32);
impl<'b> S<'b> {
    fn bar<'a>(&'a mut self) -> &'a mut &'a i32 { let ref mut x = self.0; x }
}
fn main() { println!("live"); }"#,
                expect: Expect::Invalid("lifetime"),
            },
            Case {
                name: "mut_ref_result_invariant_fn",
                source: r#"fn f<'a, 'b>(x: &'a mut &'b i32) -> &'a mut &'a i32 { x }
fn main() { println!("live"); }"#,
                expect: Expect::Invalid("lifetime"),
            },
            Case {
                name: "mut_ref_result_exact_region",
                source: r#"struct S<'b>(&'b i32);
impl<'b> S<'b> { fn bar<'a>(&'a mut self) -> &'a mut &'b i32 { &mut self.0 } }
fn main() { let v = 3; let mut s = S(&v); println!("{}", **s.bar()); }"#,
                expect: Expect::Run("3\n"),
            },
            Case {
                name: "impl_anonymous_lifetime_escape",
                source: r#"struct Bar;
struct Foo<'s> { bar: &'s mut Bar }
impl Foo<'_> { fn new(bar: &mut Bar) -> Self { Foo { bar } } }
fn main() { println!("live"); }"#,
                expect: Expect::Invalid("lifetime"),
            },
            Case {
                name: "impl_anonymous_lifetime_valid",
                source: r#"struct Foo<'s> { bar: &'s u8 }
impl Foo<'_> { fn get(&self) -> u8 { *self.bar } fn same(&self) -> &u8 { self.bar } }
fn main() { let x = 4; let f = Foo { bar: &x }; println!("{} {}", f.get(), f.same()); }"#,
                expect: Expect::Run("4 4\n"),
            },
            Case {
                name: "impl_anonymous_lifetimes_distinct",
                source: r#"struct Pair<'a, 'b> { left: &'a u8, right: &'b u8 }
impl Pair<'_, '_> { fn sum(&self) -> u8 { *self.left + *self.right } }
fn main() { let x = 4; let y = 7; let p = Pair { left: &x, right: &y }; println!("{}", p.sum()); }"#,
                expect: Expect::Run("11\n"),
            },
            Case {
                name: "impl_anonymous_lifetime_with_named",
                source: r#"struct Pair<'a, 'b> { left: &'a u8, right: &'b u8 }
impl<'a> Pair<'a, '_> { fn left(&self) -> &'a u8 { self.left } }
fn main() { let x = 4; let y = 7; let p = Pair { left: &x, right: &y }; println!("{}", p.left()); }"#,
                expect: Expect::Run("4\n"),
            },
            Case {
                name: "impl_anonymous_lifetime_trait_owner",
                source: r#"struct Foo<'a> { value: &'a u8 }
trait Read<'a> { fn read(&self) -> u8; }
impl Read<'_> for Foo<'_> { fn read(&self) -> u8 { *self.value } }
fn main() { let x = 9; let f = Foo { value: &x }; println!("{}", f.read()); }"#,
                expect: Expect::Run("9\n"),
            },
            Case {
                name: "impl_anonymous_lifetime_undeclared_scope",
                source: r#"struct Foo<'a> { value: &'a u8 }
impl Foo<'_> { fn bad(&self) -> &'a u8 { self.value } }
fn main() {}"#,
                expect: Expect::Invalid("lifetime"),
            },
            Case {
                name: "impl_anonymous_lifetime_not_named_alias",
                source: r#"struct Pair<'a, 'b> { left: &'a u8, right: &'b u8 }
impl<'a> Pair<'a, '_> { fn bad(&self) -> &'a u8 { self.right } }
fn main() {}"#,
                expect: Expect::Invalid("lifetime"),
            },
            Case {
                name: "impl_anonymous_lifetime_excess_arity",
                source: r#"struct Foo<'a> { value: &'a u8 }
impl Foo<'_, '_> { fn read(&self) -> u8 { *self.value } }
fn main() {}"#,
                expect: Expect::Invalid("lifetime"),
            },
            Case {
                name: "consistent_ref_binding",
                source: r#"fn f(x: (i32, i32)) -> i32 { match x { (ref a, 1) | (1, ref a) => *a, _ => 0 } }
fn main() { println!("{} {}", f((5, 1)), f((2, 2))); }"#,
                expect: Expect::Run("5 0\n"),
            },
            Case {
                name: "nonexhaustive_unused",
                source: r#"fn unused(c: char) -> i32 { match c { 'a' | 'b' => 1 } }
fn main() { println!("live"); }"#,
                expect: Expect::Invalid("E0004"),
            },
        ],
    );
}

/// An or-arm split into rows (range | literal, binding rows) checks its body
/// once: each source call is recorded a single time and runs once.
#[test]
fn split_or_arm_bodies_with_calls_match_stock() {
    oracle::check(
        "or-arm-calls",
        &[
            Case {
                name: "range_and_literal_rows_with_calls",
                source: r#"fn f(c: char, v: &mut Vec<char>) -> usize {
    match c {
        '0'..='9' | '.' => { v.push(c); let s: String = v.iter().collect(); s.len() }
        'a' | 'b' => { v.clear(); v.len() }
        _ => 99,
    }
}
fn main() {
    let mut v = Vec::new();
    let a = f('1', &mut v);
    let b = f('.', &mut v);
    let c = f('a', &mut v);
    let d = f('z', &mut v);
    println!("{} {} {} {} {}", a, b, c, d, v.len());
}"#,
                expect: Expect::Run("1 2 0 99 0\n"),
            },
            Case {
                name: "binding_rows_with_format_calls",
                source: r#"enum T { A(i32), B(i32), C }
fn f(t: T) -> String { match t { T::A(x) | T::B(x) => format!("v{}", x + 1), T::C => String::from("c") } }
fn main() { println!("{} {} {}", f(T::A(3)), f(T::B(4)), f(T::C)); }"#,
                expect: Expect::Run("v4 v5 c\n"),
            },
        ],
    );
}

/// String literal patterns compare `&str` contents at any depth.
#[test]
fn string_literal_patterns_match_stock() {
    oracle::check(
        "str-patterns",
        &[
            Case {
                name: "contents_nested_or_guard",
                source: r#"#[derive(Debug)] enum Dir { N, S }
#[derive(Debug)] enum Cmd { Go(Dir), Take(String), Look }
fn parse(s: &str) -> Option<Cmd> {
    let mut w = s.split_whitespace();
    Some(match (w.next()?, w.next()) {
        ("go", Some(d)) => Cmd::Go(match d { "n" | "north" => Dir::N, "s" => Dir::S, _ => return None }),
        ("take", Some(x)) => Cmd::Take(x.into()),
        ("look", None) => Cmd::Look,
        _ => return None,
    })
}
fn kind(s: &str) -> u8 { match s { "" => 0, "a" => 1, "ab" => 2, "é" => 3, other => other.len() as u8 } }
fn opt(o: Option<&str>) -> i32 { match o { Some("hi") => 1, Some("") => 4, Some(_) => 2, None => 3 } }
fn rr(s: &&str) -> bool { match *s { "x" => true, _ => false } }
fn owned(s: &String) -> i32 { match s.as_str() { "one" => 1, "two" => 2, _ => 0 } }
fn guard(s: &str, n: i32) -> i32 { match s { "k" if n > 0 => n, "k" => -1, _ => 0 } }
fn main() {
    for l in ["go n", "go north", "go s", "go q", "take key", "look", "look x", ""] { println!("{:?}", parse(l)); }
    println!("{} {} {} {} {}", kind(""), kind("a"), kind("ab"), kind("é"), kind("abcd"));
    println!("{} {} {} {}", opt(Some("hi")), opt(Some("")), opt(Some("x")), opt(None));
    println!("{} {}", rr(&"x"), rr(&"y"));
    println!("{} {} {}", owned(&String::from("one")), owned(&"two".to_string()), owned(&String::new()));
    println!("{} {} {}", guard("k", 5), guard("k", 0), guard("z", 1));
    let p = ("a", 3);
    if let ("a", n) = p { println!("{n}"); }
    let v = "yes";
    println!("{}", matches!(v, "yes" | "y"));
    let Some("q") = Some("q") else { return };
    while let "never" = v { break; }
    println!("done");
}"#,
                expect: Expect::Run(
                    "Some(Go(N))\nSome(Go(N))\nSome(Go(S))\nNone\nSome(Take(\"key\"))\nSome(Look)\nNone\nNone\n0 1 2 3 4\n1 4 2 3\ntrue false\n1 2 0\n5 -1 0\n3\ntrue\ndone\n",
                ),
            },
            Case {
                name: "non_exhaustive",
                source: r#"fn main() { let s = "a"; let n = match s { "a" => 1, "b" => 2 }; println!("{n}"); }"#,
                expect: Expect::Invalid("E0004"),
            },
            Case {
                name: "non_str_scrutinee",
                source: r#"fn main() { let n = 3; let r = match n { "a" => 1, _ => 2 }; println!("{r}"); }"#,
                expect: Expect::Invalid("E0308"),
            },
            Case {
                name: "string_scrutinee",
                source: r#"fn f(s: String) -> i32 { match s { "a" => 1, _ => 2 } }
fn main() { println!("{}", f(String::new())); }"#,
                expect: Expect::Invalid("E0308"),
            },
            Case {
                name: "no_reference_peeling",
                source: r#"fn f(s: &&str) -> bool { match s { "x" => true, _ => false } }
fn main() { println!("{}", f(&"x")); }"#,
                expect: Expect::Invalid("E0308"),
            },
        ],
    );
}

/// A guard sees a by-value non-Copy binding through a shared reference. The
/// move happens only once the guard passes, so a failed guard leaves the
/// scrutinee whole for later arms.
#[test]
fn guarded_by_value_bindings_match_stock() {
    oracle::check(
        "guard-by-value",
        &[
            Case {
                name: "moves_after_guard",
                source: r#"use std::cell::Cell;
struct D<'a>(&'a Cell<i32>, i32);
impl<'a> Drop for D<'a> { fn drop(&mut self) { self.0.set(self.0.get() + 1); println!("drop {}", self.1); } }
enum E { A(String), B(String, i32), C }
enum N { W(Option<(String, i32)>), Z }
fn fail_then_later(e: E) -> String { match e { E::A(s) if s.len() > 3 => format!("long {s}"), E::A(s) => format!("short {s}"), E::B(s, n) if n > 0 => { let t = s; format!("pos {t} {n}") } E::B(s, _) => s, E::C => String::from("c") } }
fn whole(s: String) -> String { match s { t if t.is_empty() => String::from("empty"), t if t == "x" => t + "!", t => t } }
fn by_ref(e: &E) -> usize { match e { E::A(s) if s.len() > 3 => s.len(), E::A(s) => s.len() + 100, E::B(s, n) if *n > 0 => s.len(), E::B(..) => 0, E::C => 1 } }
fn by_mut(e: &mut E) -> usize { match e { E::A(s) if s.len() > 3 => { s.push('!'); s.len() } E::A(s) => { s.push('?'); s.len() + 100 } _ => 0 } }
fn nested(n: N) -> String { match n { N::W(Some((s, k))) if k > 1 => s, N::W(Some((s, _))) => s + "-low", N::W(None) => String::from("none"), N::Z => String::from("z") } }
fn guard_borrows(v: Vec<String>, x: String) -> String { match x { y if v.contains(&y) && y == "key" => format!("has {y}"), y => format!("no {y}") } }
fn drops(c: &Cell<i32>, pass: bool) -> i32 { let o = Some(D(c, 7)); match o { Some(d) if pass && d.1 == 7 => { println!("body {}", d.1); 1 } Some(d) => { println!("later {}", d.1); 2 } None => 3 } }
fn main() {
    println!("{}", fail_then_later(E::A("hello".into())));
    println!("{}", fail_then_later(E::A("hi".into())));
    println!("{}", fail_then_later(E::B("b".into(), 2)));
    println!("{}", fail_then_later(E::B("b".into(), -2)));
    println!("{}", fail_then_later(E::C));
    println!("{} {} {}", whole(String::new()), whole("x".into()), whole("yz".into()));
    println!("{} {} {} {} {}", by_ref(&E::A("hello".into())), by_ref(&E::A("hi".into())), by_ref(&E::B("abc".into(), 1)), by_ref(&E::B("abc".into(), 0)), by_ref(&E::C));
    let mut a = E::A("hello".into()); let mut b = E::A("hi".into());
    println!("{} {}", by_mut(&mut a), by_mut(&mut b));
    if let E::A(s) = &a { println!("{s}"); }
    if let E::A(s) = &b { println!("{s}"); }
    println!("{} {} {} {}", nested(N::W(Some(("p".into(), 5)))), nested(N::W(Some(("q".into(), 0)))), nested(N::W(None)), nested(N::Z));
    println!("{}", guard_borrows(vec!["key".to_string()], "key".into()));
    println!("{}", guard_borrows(vec!["key".to_string()], "rose".into()));
    let c = Cell::new(0);
    println!("r {}", drops(&c, true)); println!("count {}", c.get());
    println!("r {}", drops(&c, false)); println!("count {}", c.get());
}"#,
                expect: Expect::Run(
                    "long hello\nshort hi\npos b 2\nb\nc\nempty x! yz\n5 102 3 0 1\n6 103\nhello!\nhi?\np q-low none z\nhas key\nno rose\nbody 7\ndrop 7\nr 1\ncount 1\nlater 7\ndrop 7\nr 2\ncount 2\n",
                ),
            },
            Case {
                name: "or_rows_move_after_guard",
                source: r#"use std::cell::Cell;
struct D<'a>(&'a Cell<i32>, i32);
impl<'a> Drop for D<'a> { fn drop(&mut self) { self.0.set(self.0.get() + 1); println!("drop {}", self.1); } }
enum E<'a> { A(D<'a>, i32), B(i32, D<'a>), C }
fn f(e: E<'_>, pass: bool) -> i32 { match e { E::A(d, n) | E::B(n, d) if pass && n > 0 => { let k = d; println!("body {} {n}", k.1); 1 } E::A(d, _) | E::B(_, d) => { println!("later {}", d.1); 2 } E::C => 3 } }
fn g(o: Option<Result<String, String>>) -> String { match o { Some(Ok(s) | Err(s)) if s.len() > 1 => s + "+", Some(Ok(s)) => s + "ok", Some(Err(s)) => s + "err", None => String::new() } }
fn main() {
    let c = Cell::new(0);
    println!("{}", f(E::A(D(&c, 1), 5), true)); println!("{}", f(E::B(5, D(&c, 2)), true));
    println!("{}", f(E::A(D(&c, 3), 5), false)); println!("{}", f(E::B(0, D(&c, 4)), true)); println!("{}", f(E::C, true));
    println!("count {}", c.get());
    println!("{} {} {} {} [{}]", g(Some(Ok("ab".into()))), g(Some(Err("cd".into()))), g(Some(Ok("a".into()))), g(Some(Err("c".into()))), g(None));
}"#,
                expect: Expect::Run(
                    "body 1 5\ndrop 1\n1\nbody 2 5\ndrop 2\n1\nlater 3\ndrop 3\n2\nlater 4\ndrop 4\n2\n3\ncount 4\nab+ cd+ aok cerr []\n",
                ),
            },
            Case {
                name: "move_in_guard",
                source: r#"fn take(s: String) -> bool { s.len() > 1 }
fn main() { let o = Some(String::from("ab")); let r = match o { Some(s) if take(s) => 1, Some(_) => 2, None => 3 }; println!("{r}"); }"#,
                expect: Expect::Invalid("E0507"),
            },
            Case {
                name: "assign_in_guard",
                source: r#"fn main() { let o = Some(String::from("ab")); let r = match o { Some(s) if { s = String::new(); true } => s.len(), Some(_) => 2, None => 3 }; println!("{r}"); }"#,
                expect: Expect::Invalid("E0594"),
            },
        ],
    );
}
