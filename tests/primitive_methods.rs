//! Stock rustc is an external execution and rejection oracle only.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/oracle.rs"]
mod oracle;
use closure_oracle::pinned_stock as stock_rustc;
use oracle::{Case, Expect};

#[allow(dead_code)]
#[path = "support/closure_oracle.rs"]
mod closure_oracle;

fn parse_oracle(tag: &str, cases: &[closure_oracle::Case]) {
    closure_oracle::check_with_compiler(
        tag,
        cases,
        std::path::Path::new(&stock_rustc::stock_rustc()),
    );
}

#[test]
fn vecdeque_owned_array_conversion_matches_stock() {
    use closure_oracle::{Case, Expect};
    parse_oracle(
        "vecdeque-owned-array",
        &[Case {
            name: "empty_duplicates_and_owned_drop_order",
            source: r#"use std::collections::VecDeque;
struct Value(String);
impl Drop for Value { fn drop(&mut self) { println!("drop {}", self.0); } }
fn convert<T,const N:usize>(items:[T;N])->VecDeque<T> { VecDeque::from(items) }
fn main() {
    let empty: VecDeque<String> = convert([]);
    println!("empty {}",empty.len());
    let mut queue = VecDeque::from([3i32,3i32,7i32]);
    println!("{:?} {:?} {:?}",queue.pop_front(),queue.pop_front(),queue.pop_front());
    let mut owned = convert([Value("first".to_string()),Value("second".to_string())]);
    drop(owned.pop_front());
    println!("remaining {}",owned.len());
}"#,
            expect: Expect::Run(
                "empty 0\nSome(3) Some(3) Some(7)\ndrop first\nremaining 1\ndrop second\n",
            ),
        }],
    );
}

#[test]
fn vecdeque_owned_array_rejects_unused_move() {
    use closure_oracle::{Case, Expect};
    parse_oracle(
        "vecdeque-array-move",
        &[Case {
            name: "unused_moved_array_is_rejected",
            source: r#"use std::collections::VecDeque;
fn unused(){let items=["owned".to_string()];let _queue=VecDeque::from(items);drop(items);}
fn main(){}"#,
            expect: Expect::Invalid("E0382"),
        }],
    );
}

#[test]
fn vecdeque_owned_array_rejects_unused_escape() {
    use closure_oracle::{Case, Expect};
    parse_oracle(
        "vecdeque-array-escape",
        &[Case {
            name: "unused_borrowed_array_escape_is_rejected",
            source: r#"use std::collections::VecDeque;
fn unused<'a>()->VecDeque<&'a String>{let value="local".to_string();VecDeque::from([&value])}
fn main(){}"#,
            expect: Expect::Invalid("E0515"),
        }],
    );
}

#[test]
fn vecdeque_owned_array_frontier_demands_are_general() {
    use closure_oracle::{Case, Expect};
    parse_oracle(
        "associated-nominal-frontier",
        &[
            Case {
                name: "renamed_trait_competing_nominal_argument",
                source: r#"mod api { pub trait Make<S> { fn make(source:S)->Self; } }
use api::Make as Assemble;
struct Holder<T> { value:T }
impl<T:Copy> Assemble<[T;1]> for Holder<T> {
    fn make(source:[T;1])->Self { Holder { value:source[0] } }
}
impl<T> Assemble<Vec<T>> for Holder<T> {
    fn make(_source:Vec<T>)->Self { loop {} }
}
fn main(){let holder=Holder::<i32>::make([7i32]);println!("{}",holder.value);}"#,
                expect: Expect::Run("7\n"),
            },
            Case {
                name: "failed_bound_nominal_competitor_does_not_add_ambiguity",
                source: r#"trait Make<S> { fn make(source:S)->Self; }
trait Other<S> { fn make(source:S)->Self; }
trait Disabled {}
struct Holder<T> { value:T }
impl<T:Copy> Make<[T;1]> for Holder<T> {
    fn make(source:[T;1])->Self { Holder { value:source[0] } }
}
impl<T:Disabled> Other<Vec<T>> for Holder<T> {
    fn make(_source:Vec<T>)->Self { loop {} }
}
fn main(){let holder=Holder::<i32>::make([7i32]);println!("{}",holder.value);}"#,
                expect: Expect::Run("7\n"),
            },
            Case {
                name: "failed_candidate_cannot_demand_illformed_nominal",
                source: r#"trait Make<S> { fn make(source:S)->Self; }
trait Enabled {}
struct Needs<T:Enabled>(T);
struct Holder<T> { value:T }
impl<T:Copy> Make<[T;1]> for Holder<T> {
    fn make(source:[T;1])->Self { Holder { value:source[0] } }
}
impl<T:Enabled> Make<Needs<T>> for Holder<T> {
    fn make(_source:Needs<T>)->Self { loop {} }
}
fn main(){let holder=Holder::<i32>::make([7i32]);println!("{}",holder.value);}"#,
                expect: Expect::Run("7\n"),
            },
            Case {
                name: "unused_other_trait_nominal_competitor_remains_ambiguous",
                source: r#"trait Make<S> { fn make(source:S)->Self; }
trait Other<S> { fn make(source:S)->Self; }
struct Holder<T> { value:T }
impl<T:Copy> Make<[T;1]> for Holder<T> {
    fn make(source:[T;1])->Self { Holder { value:source[0] } }
}
impl<T> Other<Vec<T>> for Holder<T> {
    fn make(_source:Vec<T>)->Self { loop {} }
}
fn unused(){let _=Holder::<i32>::make([7i32]);} fn main(){}"#,
                expect: Expect::Invalid("E0034"),
            },
        ],
    );
}

#[test]
fn rvalue_and_dynamic_index_receivers_match_stock() {
    use closure_oracle::{Case, Expect};
    parse_oracle(
        "rvalue-dynamic-index-receivers",
        &[
            Case {
                name: "rvalue_dynamic_index_receivers",
                source: r#"
#[derive(Debug, Clone)]
#[allow(dead_code)]
enum Ins { Push(i64), Halt }
struct Counter { n: u32 }
impl Counter {
    fn bump(&mut self) -> u32 { self.n += 1; self.n }
    fn get(&self) -> u32 { self.n }
}
impl Drop for Counter { fn drop(&mut self) { println!("drop {}", self.n); } }
fn make(n: u32) -> Counter { Counter { n } }
fn fetch(prog: &[Ins], pc: usize) -> Result<Ins, String> {
    let ins = prog.get(pc).ok_or("pc out of range")?.clone();
    Ok(ins)
}
fn first_len(v: &[String]) -> usize { v.first().unwrap().len() }
fn lookup(vars: &std::collections::HashMap<String, i64>, v: &str) -> Result<i64, String> {
    let x = *vars.get(v).ok_or(format!("undefined {v}"))?;
    Ok(x + 1)
}
fn main() {
    let mut vars = std::collections::HashMap::new();
    vars.insert(String::from("a"), 41i64);
    println!("{:?} {:?}", lookup(&vars, "a"), lookup(&vars, "b"));
    let prog = vec![Ins::Push(3), Ins::Halt];
    println!("{:?} {:?}", fetch(&prog, 0), fetch(&prog, 5));
    println!("{}", first_len(&[String::from("abc")]));
    let x = make(1).bump();
    println!("after {}", x);
    let v = vec![make(5), make(7)];
    let i = 1;
    println!("{}", v[i].get());
    let mut w = vec![vec![1u32], vec![2u32]];
    let j = 1;
    w[j].push(9);
    println!("{:?}", w);
}
"#,
                expect: Expect::Run(
                    "Ok(42) Err(\"undefined b\")\nOk(Push(3)) Err(\"pc out of range\")\n3\ndrop 2\nafter 2\n7\n[[1], [2, 9]]\ndrop 5\ndrop 7\n",
                ),
            },
            Case {
                name: "rvalue_receiver_borrow_escapes_statement_unused",
                source: r#"
fn make() -> String { String::from("hello") }
#[allow(dead_code)]
fn unused() {
    let r: &str = make().as_str();
    println!("{}", r);
}
fn main() {}
"#,
                expect: Expect::Invalid("E0716"),
            },
            Case {
                name: "deref_try_root_moves_string_unused",
                source: r#"
#[allow(dead_code)]
fn take(v: &[String]) -> Result<String, String> {
    let s = *v.first().ok_or(String::from("empty"))?;
    Ok(s)
}
fn main() {}
"#,
                expect: Expect::Invalid("E0507"),
            },
            Case {
                name: "deref_match_root_moves_string_unused",
                source: r#"
#[allow(dead_code)]
fn take(o: Option<&String>) -> String {
    let s = *match o { Some(r) => r, None => return String::new() };
    s
}
fn main() {}
"#,
                expect: Expect::Invalid("E0507"),
            },
        ],
    );
}

#[test]
fn unchanged_json_parser_matches_stock() {
    parse_oracle(
        "unchanged-json-parser",
        &[closure_oracle::Case {
            name: "03_json_parser",
            source: include_str!("milestones/m3/03_json_parser.rs"),
            expect: closure_oracle::Expect::Run(include_str!(
                "milestones/m3/03_json_parser.stdout"
            )),
        }],
    );
}

#[test]
fn container_clone_and_equality_match_stock() {
    use closure_oracle::{Case, Expect};
    parse_oracle(
        "container-clone-equality",
        &[
            Case {
                name: "recursive_box_btreemap_hashmap_clone_eq",
                source: r#"
use std::collections::{BTreeMap, HashMap};
#[derive(Debug, Clone, PartialEq)]
enum J { N, B(bool), A(Vec<J>), O(BTreeMap<String, J>), X(Box<J>) }
fn main() {
    let mut m = BTreeMap::new();
    m.insert("k".to_string(), J::A(vec![J::N, J::B(true)]));
    m.insert("a".to_string(), J::X(Box::new(J::N)));
    let j = J::O(m);
    let k = j.clone();
    println!("{:?} {}", k, k == j);
    let mut m2 = BTreeMap::new();
    m2.insert("k".to_string(), J::N);
    println!("{} {}", J::O(m2) == j, J::X(Box::new(J::B(false))) != J::X(Box::new(J::B(true))));
    let b: Box<i32> = Box::new(5);
    let c = b.clone();
    println!("{} {}", c, b == c);
    let mut h: HashMap<u8, String> = HashMap::new();
    h.insert(1, "x".to_string());
    h.insert(2, "y".to_string());
    let g = h.clone();
    let mut d = HashMap::new();
    d.insert(2u8, "y".to_string());
    d.insert(1u8, "x".to_string());
    println!("{} {} {}", g == h, d == h, g.len());
    d.insert(3, "z".to_string());
    println!("{}", d == h);
}
"#,
                expect: Expect::Run(
                    "O({\"a\": X(N), \"k\": A([N, B(true)])}) true\nfalse true\n5 true\ntrue true 2\nfalse\n",
                ),
            },
            Case {
                name: "derive_clone_non_clone_field_unused",
                source: r#"
struct NoClone;
#[derive(Clone)]
struct S { a: i32, b: NoClone }
fn main() {}
"#,
                expect: Expect::Invalid("E0277"),
            },
            Case {
                name: "box_eq_without_partial_eq_unused",
                source: r#"
struct NoEq;
fn unused(a: Box<NoEq>, b: Box<NoEq>) -> bool { a == b }
fn main() {}
"#,
                expect: Expect::Invalid("E0369"),
            },
            Case {
                name: "btreemap_clone_non_clone_value_unused",
                source: r#"
use std::collections::BTreeMap;
struct NoClone;
fn unused(m: &BTreeMap<i32, NoClone>) -> BTreeMap<i32, NoClone> { m.clone() }
fn main() {}
"#,
                expect: Expect::Invalid("E0308"),
            },
        ],
    );
}

#[test]
fn method_body_parse_inference_matches_stock() {
    use closure_oracle::{Case, Expect};
    parse_oracle(
        "method-body-parse-inference",
        &[
            Case {
                name: "method_body_parse_chains",
                source: r#"
#[derive(Debug)]
enum J { Num(f64), Int(i64) }
struct P { s: String }
impl P {
    fn num(&self) -> Result<J, String> { self.s.as_str().parse().map(J::Num).map_err(|e| format!("bad {e}")) }
    fn int(&self, t: &str) -> Result<J, String> { t.parse().map(|n| J::Int(n)).map_err(|e| format!("int {e}")) }
    fn float(t: &str) -> Result<f64, String> { t.parse().map_err(|e| format!("f {e}")) }
}
fn main() {
    let p = P { s: "2.5".into() };
    println!("{:?} {:?}", p.num(), P { s: "z".into() }.num());
    println!("{:?} {:?}", p.int("7"), p.int("x"));
    println!("{:?} {:?}", P::float("1e2"), P::float(""));
}
"#,
                expect: Expect::Run(
                    "Ok(Num(2.5)) Err(\"bad invalid float literal\")\nOk(Int(7)) Err(\"int invalid digit found in string\")\nOk(100.0) Err(\"f cannot parse float from empty string\")\n",
                ),
            },
            Case {
                name: "method_body_parse_recursive_enum",
                source: r#"
#[derive(Debug)]
enum T { Leaf(i64), Node(Vec<T>) }
struct R<'a> { parts: Vec<&'a str> }
impl<'a> R<'a> {
    fn tree(&self) -> Result<T, String> {
        let mut kids = Vec::new();
        for p in &self.parts { kids.push(p.parse().map(T::Leaf).map_err(|e| format!("{p}: {e}"))?); }
        Ok(T::Node(kids))
    }
}
fn main() {
    println!("{:?}", R { parts: vec!["1", "2"] }.tree());
    println!("{:?}", R { parts: vec!["1", "x"] }.tree());
}
"#,
                expect: Expect::Run(
                    "Ok(Node([Leaf(1), Leaf(2)]))\nErr(\"x: invalid digit found in string\")\n",
                ),
            },
            Case {
                name: "reference_carrying_recursive_enum_outlived_unused",
                source: r#"
#[allow(dead_code)]
enum L<'a> { Nil, C(&'a str, Box<L<'a>>) }
#[allow(dead_code)]
fn f() -> usize {
    let l;
    { let s = String::from("x"); l = L::C(&s, Box::new(L::Nil)); }
    match l { L::C(w, _) => w.len(), L::Nil => 0 }
}
fn main() {}
"#,
                expect: Expect::Invalid("E0597"),
            },
            Case {
                name: "char_from_u32_and_slice_affixes",
                source: r#"
fn main() {
    println!("{:?} {:?} {:?}", char::from_u32(0xe9), char::from_u32(0xD800), char::from_u32(0x110000));
    println!("{:?} {:?}", char::from_u32(u32::from_str_radix("0041", 16).unwrap()), char::from_digit(11, 16));
    let b: &[u8] = b"null,";
    println!("{} {} {} {}", b.starts_with(b"nul"), b.ends_with(b","), b.starts_with(b"nulll,"), [1, 2, 3].ends_with(&[2, 3]));
}
"#,
                expect: Expect::Run(
                    "Some('é') None None\nSome('A') Some('b')\ntrue true false true\n",
                ),
            },
            Case {
                name: "char_from_u32_wrong_argument_type_unused",
                source: "fn main() { let _ = char::from_u32(65u64); }\n",
                expect: Expect::Invalid("E0308"),
            },
            Case {
                name: "method_body_parse_error_type_mismatch_unused",
                source: "struct P;\n#[allow(dead_code)]\nimpl P { fn a(&self, s: &str) -> Result<u8, String> { s.parse().map_err(|e| e) } }\nfn main() {}\n",
                expect: Expect::Invalid("E0308"),
            },
            Case {
                name: "method_body_parse_unit_target_unused",
                source: "struct P;\n#[allow(dead_code)]\nimpl P { fn a(&self, s: &str) -> Result<(), String> { s.parse().map_err(|e| format!(\"{e}\")) } }\nfn main() {}\n",
                expect: Expect::Invalid("E0277"),
            },
            Case {
                name: "method_body_parse_unconstrained_unused",
                source: "struct P;\n#[allow(dead_code)]\nimpl P { fn a(&self, s: &str) { let _x = s.parse().unwrap(); } }\nfn main() {}\n",
                expect: Expect::Invalid("E0284"),
            },
            Case {
                name: "method_body_parse_annotated_mismatch_unused",
                source: "struct P;\n#[allow(dead_code)]\nimpl P { fn a(&self, s: &str) -> Result<f64, String> { let r: Result<i32, String> = s.parse().map_err(|e| format!(\"{e}\")); r } }\nfn main() {}\n",
                expect: Expect::Invalid("E0308"),
            },
        ],
    );
}

#[test]
fn callable_value_arity_codes_match_stock() {
    // Stock: E0061 for function item and function pointer values (including
    // tuple constructors), E0057 only for closures.
    for (source, code) in [
        (
            "#[allow(dead_code)] enum E { A(i32) }\n#[allow(dead_code)] fn f() { let g = E::A; let _ = g(1, 2); }\nfn main() {}\n",
            "E0061",
        ),
        (
            "#[allow(dead_code)] fn g(x: i32) -> i32 { x }\n#[allow(dead_code)] fn f() { let h: fn(i32) -> i32 = g; let _ = h(1, 2); }\nfn main() {}\n",
            "E0061",
        ),
        (
            "#[allow(dead_code)] fn f() { let h = |x: i32| x; let _ = h(1, 2); }\nfn main() {}\n",
            "E0057",
        ),
    ] {
        let error = justrust_engine::compiler::compile_observed(source.to_string(), 1, true)
            .err()
            .unwrap_or_else(|| panic!("accepted invalid arity: {source}"));
        let text = format!("{error:?}");
        assert!(text.contains(code), "expected {code} for {source}: {text}");
    }
}

#[test]
fn tuple_constructor_values_match_stock() {
    use closure_oracle::{Case, Expect};
    parse_oracle(
        "tuple-constructor-values",
        &[
            Case {
                name: "tuple_constructor_values",
                source: r#"
#[derive(Debug, Clone, PartialEq)]
enum J { Num(f64), Str(String), Pair(i32, char) }
#[derive(Debug)]
struct W(i32, u8);
#[derive(Debug)]
enum G<T> { A(T), B }
mod m { #[derive(Debug)] pub struct Id(pub u32); }
fn num(s: &str) -> Result<J, String> { s.parse().map(J::Num).map_err(|e| format!("bad {e}")) }
fn ap<F: Fn(i32) -> J>(f: F) -> J { f(7) }
fn twice<A, B>(f: impl Fn(A) -> B, a: A, b: A) -> (B, B) { (f(a), f(b)) }
fn main() {
    println!("{:?} {:?}", num("1.5"), num("x"));
    let v: Vec<Option<i32>> = vec![1, 2].into_iter().map(Some).collect();
    println!("{v:?}");
    let words: Vec<J> = ["a", "bc"].iter().map(|s| s.to_string()).map(J::Str).collect();
    println!("{words:?}");
    let mk = W;
    let ws: Vec<W> = vec![(1, 2u8), (3, 4)].into_iter().map(|(a, b)| mk(a, b)).collect();
    println!("{ws:?}");
    let gs: Vec<G<String>> = vec!["x".to_string()].into_iter().map(G::A).collect();
    println!("{gs:?} {:?}", G::<u8>::B);
    let p: fn(i32, char) -> J = J::Pair;
    println!("{:?} {:?}", p(3, 'z'), ap(|n| J::Num(n as f64)));
    let ids: Vec<m::Id> = (1..4).map(m::Id).collect();
    println!("{ids:?}");
    println!("{:?}", twice(Some, 1, 2));
    let r: Result<i32, String> = Err("e".into());
    println!("{:?}", r.map_err(J::Str).unwrap_or_else(|j| if j == J::Str("e".into()) { -1 } else { 0 }));
    let fs = [J::Num as fn(f64) -> J];
    println!("{:?}", fs[0](2.5));
}
"#,
                expect: Expect::Run(
                    "Ok(Num(1.5)) Err(\"bad invalid float literal\")\n[Some(1), Some(2)]\n[Str(\"a\"), Str(\"bc\")]\n[W(1, 2), W(3, 4)]\n[A(\"x\")] B\nPair(3, 'z') Num(7.0)\n[Id(1), Id(2), Id(3)]\n(Some(1), Some(2))\n-1\nNum(2.5)\n",
                ),
            },
            Case {
                name: "constructor_value_wrong_arity_unused",
                source: r#"
#[allow(dead_code)] enum E { A(i32) }
#[allow(dead_code)] fn f() { let g = E::A; let _ = g(1, 2); }
fn main() {}
"#,
                expect: Expect::Invalid("E0061"),
            },
            Case {
                name: "struct_variant_as_value_unused",
                source: r#"
#[allow(dead_code)] enum E { S { x: i32 } }
#[allow(dead_code)] fn f() { let _g = E::S; }
fn main() {}
"#,
                expect: Expect::Invalid("E0533"),
            },
            Case {
                name: "private_tuple_struct_constructor_value_unused",
                source: r#"
mod m { pub struct P(i32); pub fn mk() -> P { P(1) } }
#[allow(dead_code)] fn f() { let _g = m::P; }
fn main() { let _ = m::mk(); }
"#,
                expect: Expect::Invalid("E0603"),
            },
            Case {
                name: "constructor_value_wrong_pointer_type_unused",
                source: r#"
#[allow(dead_code)] enum E { A(i32) }
#[allow(dead_code)] fn f() -> fn(u8) -> E { E::A }
fn main() {}
"#,
                expect: Expect::Invalid("E0308"),
            },
        ],
    );
}

#[test]
fn unchanged_vm_interpreter_matches_stock() {
    parse_oracle(
        "unchanged-vm-interpreter",
        &[closure_oracle::Case {
            name: "05_vm_interpreter",
            source: include_str!("milestones/m3/05_vm_interpreter.rs"),
            expect: closure_oracle::Expect::Run(include_str!(
                "milestones/m3/05_vm_interpreter.stdout"
            )),
        }],
    );
}

#[test]
fn unchanged_matrix_generic_num_matches_stock() {
    parse_oracle(
        "unchanged-matrix-generic-num",
        &[closure_oracle::Case {
            name: "08_matrix_generic_num",
            source: include_str!("milestones/m3/08_matrix_generic_num.rs"),
            expect: closure_oracle::Expect::Run(include_str!(
                "milestones/m3/08_matrix_generic_num.stdout"
            )),
        }],
    );
}

#[test]
fn generic_local_sequence_indexed_write_matches_stock() {
    use closure_oracle::{Case, Expect};
    parse_oracle(
        "generic-local-sequence-indexed-write",
        &[
            Case {
                name: "generic_local_sequence_indexed_write",
                source: r#"
use std::ops::{Add, Mul};
use std::fmt::Display;
trait Num: Copy + Add<Output = Self> + Mul<Output = Self> + Display { fn zero() -> Self; fn one() -> Self; }
impl Num for i64 { fn zero() -> Self { 0 } fn one() -> Self { 1 } }
impl Num for f64 { fn zero() -> Self { 0.0 } fn one() -> Self { 1.0 } }
struct Sq<T> { n: usize, d: Vec<T> }
impl<T: Num> Sq<T> {
    fn ident(n: usize) -> Self { let mut d = vec![T::zero(); n * n]; for i in 0..n { d[i * n + i] = T::one(); } Sq { n, d } }
    fn mul(&self, o: &Self) -> Self {
        let n = self.n; let mut d = vec![T::zero(); n * n];
        for i in 0..n { for j in 0..n { let mut s = T::zero(); for k in 0..n { s = s + self.d[i * n + k] * o.d[k * n + j]; } d[i * n + j] = s; } }
        Sq { n, d }
    }
    fn show(&self) -> String { let mut out = String::new(); for x in &self.d { out.push_str(&format!("{x} ")); } out }
}
fn fill<T: Clone>(mut v: Vec<T>, x: T) -> Vec<T> { let i = v.len() - 1; v[i] = x; v }
// Generic `Vec<&'a T>` element stores (`fn put<'a, T>(mut v: Vec<&'a T>, x: &'a T)`) are
// blocked by the equality-RHS limitation and rejected with an explicit diagnostic.
fn main() {
    let a = Sq { n: 2, d: vec![1i64, 1, 1, 0] };
    println!("{}", a.mul(&a).mul(&Sq::ident(2)).show());
    let b = Sq { n: 2, d: vec![0.5f64, 2.0, 1.0, 0.0] };
    println!("{}", b.mul(&Sq::<f64>::ident(2)).mul(&b).show());
    println!("{:?}", fill(vec![String::from("a"), String::from("b")], String::from("z")));
}
"#,
                expect: Expect::Run("2 1 1 1 \n2.25 1 0.5 2 \n[\"a\", \"z\"]\n"),
            },
            Case {
                name: "generic_local_sequence_short_reference_store_unused",
                source: r#"
#[allow(dead_code)]
fn f<'a, T: Copy>(x: &'a T, y: &'static T) -> Vec<&'static T> { let mut d: Vec<&'static T> = Vec::new(); d.push(y); d[0] = x; d }
fn main() {}
"#,
                expect: Expect::Invalid("lifetime may not live long enough"),
            },
            Case {
                name: "generic_local_sequence_store_then_use_moved_unused",
                source: r#"
#[allow(dead_code)]
fn f<T: Default>(n: usize, v: T) -> Vec<T> { let mut d: Vec<T> = Vec::new(); for _ in 0..n { d.push(T::default()); } d[0] = v; drop(v); d }
fn main() {}
"#,
                expect: Expect::Invalid("E0382"),
            },
            Case {
                name: "by_value_param_sequence_short_reference_returned_unused",
                source: r#"
#[allow(dead_code)]
fn put<'a, 'b, T>(mut v: Vec<&'a T>, x: &'b T) -> Vec<&'a T> { v[0] = x; v }
fn main() {}
"#,
                expect: Expect::Invalid("lifetime may not live long enough"),
            },
            Case {
                name: "by_value_param_sequence_store_then_use_moved_unused",
                source: r#"
#[allow(dead_code)]
fn fill<T>(mut v: Vec<T>, x: T) -> Vec<T> { v[0] = x; drop(x); v }
fn main() {}
"#,
                expect: Expect::Invalid("E0382"),
            },
        ],
    );
}

#[test]
fn fn_item_callable_reference_result_matches_stock() {
    use closure_oracle::{Case, Expect};
    parse_oracle(
        "fn-item-callable-reference-result",
        &[
            Case {
                name: "fn_item_fnmut_str_result",
                source: r#"
fn first_word(s: &str) -> &str { s.split(' ').next().unwrap() }
fn main() {
    let v: Vec<&str> = "x y\nz w".lines().map(first_word).collect();
    println!("{:?}", v);
    let t: Vec<&str> = " p \n q ".lines().map(str::trim).collect();
    println!("{:?}", t);
    let n = "hello world\na b".lines().map(first_word).map(str::len).sum::<usize>();
    println!("{}", n);
}
"#,
                expect: Expect::Run("[\"x\", \"z\"]\n[\"p\", \"q\"]\n6\n"),
            },
            Case {
                name: "fn_item_result_borrows_local_unused",
                source: r#"
fn first_word(s: &str) -> &str { s.split(' ').next().unwrap() }
#[allow(dead_code)]
fn leak() -> &'static str {
    let local = String::from("hi there");
    let v: Vec<&str> = local.lines().map(first_word).collect();
    v[0]
}
fn main() {}
"#,
                expect: Expect::Invalid("E0515"),
            },
            Case {
                name: "fn_item_result_outlives_argument",
                source: r#"
fn first_word(s: &str) -> &str { s.split(' ').next().unwrap() }
fn main() {
    let words: Vec<&str>;
    {
        let s = String::from("a b");
        words = s.lines().map(first_word).collect();
    }
    println!("{:?}", words);
}
"#,
                expect: Expect::Invalid("E0597"),
            },
        ],
    );
}

#[test]
fn unchanged_grid_bfs_matches_stock() {
    parse_oracle(
        "unchanged-grid-bfs",
        &[closure_oracle::Case {
            name: "04_aoc_grid_bfs",
            source: include_str!("milestones/m3/04_aoc_grid_bfs.rs"),
            expect: closure_oracle::Expect::Run(include_str!(
                "milestones/m3/04_aoc_grid_bfs.stdout"
            )),
        }],
    );
}

#[test]
fn opaque_iterator_aggregate_preserves_capture_authority() {
    use closure_oracle::{Case, Expect};
    parse_oracle(
        "opaque-iterator-aggregate",
        &[Case {
            name: "borrowed_capture_inside_owned_adapter",
            source: r#"fn selected(values:&[i32])->impl Iterator<Item=i32>+'_ {
    [0usize,1usize,2usize,3usize].into_iter().filter_map(move |i| {
        if i < values.len() { Some(values[i]) } else { None }
    })
}
fn main(){let values=[4i32,5,6];println!("{:?}",selected(&values).collect::<Vec<i32>>());}"#,
            expect: Expect::Run("[4, 5, 6]\n"),
        }],
    );
}

#[test]
fn opaque_iterator_aggregate_rejects_unused_escape() {
    use closure_oracle::{Case, Expect};
    parse_oracle(
        "opaque-iterator-unused-escape",
        &[Case {
            name: "unused_aggregate_capture_cannot_escape_input",
            source: r#"fn selected(values:&[i32])->impl Iterator<Item=i32>+'_ {
    [0usize,1usize,2usize,3usize].into_iter().filter_map(move |i| {
        if i < values.len() { Some(values[i]) } else { None }
    })
}
fn unused()->impl Iterator<Item=i32>{let values=[4i32,5,6];selected(&values)}
fn main(){}"#,
            expect: Expect::Invalid("E0597"),
        }],
    );
}

#[test]
fn opaque_iterator_aggregate_rejects_hidden_capability() {
    use closure_oracle::{Case, Expect};
    parse_oracle(
        "opaque-iterator-hidden-capability",
        &[Case {
            name: "unused_opaque_cannot_reveal_hidden_iterator",
            source: "fn hidden()->impl Sized{vec![1i32]}fn unused(){for _ in hidden(){}}fn main(){}",
            expect: Expect::Invalid("E0277"),
        }],
    );
}

#[test]
fn primitive_inherent_then_some_is_generic_and_eager() {
    use closure_oracle::{Case, Expect};
    parse_oracle(
        "primitive-inherent-then-some",
        &[
            Case {
                name: "eager_drop_and_inherent_precedence",
                source: r#"
trait Pretend { fn then_some(self, value: i32) -> i32; }
impl Pretend for bool { fn then_some(self, value: i32) -> i32 { value + 100 } }
struct Token(i32);
impl Drop for Token { fn drop(&mut self) { println!("drop {}", self.0); } }
fn make(value: i32) -> Token { println!("make {}", value); Token(value) }
fn generic<T>(flag:bool, value:T)->Option<T>{flag.then_some(value)}
fn main(){
    let absent = false.then_some(make(1));
    println!("none {}", absent.is_none());
    let present = true.then_some(make(2));
    println!("some {}", present.is_some());
    drop(present);
    println!("{} {}", true.then_some(7).unwrap(), generic(true, String::from("owned")).unwrap());
    println!("{:?}", generic(false, (3usize, 4usize)));
    let word = String::from("borrowed");
    println!("{} {}", bool::then_some::<usize>(true, 9).unwrap(), (&true).then_some(word.as_str()).unwrap());
}"#,
                expect: Expect::Run(
                    "make 1\ndrop 1\nnone true\nmake 2\nsome true\ndrop 2\n7 owned\nNone\n9 borrowed\n",
                ),
            },
            Case {
                name: "untrusted_primitive_method_impl",
                source: "impl bool { fn forged(self) -> bool { self } } fn main(){}",
                expect: Expect::Invalid("E0390"),
            },
            Case {
                name: "unused_generic_wrong_result",
                source: "fn unused<T>(value:T)->Option<bool>{true.then_some(value)}fn main(){}",
                expect: Expect::Invalid("E0308"),
            },
            Case {
                name: "unused_missing_argument",
                source: "fn unused(){false.then_some();}fn main(){}",
                expect: Expect::Invalid("E0061"),
            },
            Case {
                name: "unused_payload_cannot_escape",
                source: "fn unused<'a>()->Option<&'a str>{let value=String::from(\"local\");true.then_some(value.as_str())}fn main(){}",
                expect: Expect::Invalid("E0515"),
            },
        ],
    );
}

#[test]
fn integer_rem_euclid_const_and_runtime_match_stock() {
    use closure_oracle::{Case, Expect};
    parse_oracle(
        "integer-rem-euclid",
        &[
            Case {
                name: "const_qualification_and_signed_edges",
                source: r#"
            const fn r8(a:i8,b:i8)->i8{a.rem_euclid(b)}
            const fn r16(a:i16,b:i16)->i16{a.rem_euclid(b)}
            const fn r32(a:i32,b:i32)->i32{a.rem_euclid(b)}
            const fn r64(a:i64,b:i64)->i64{a.rem_euclid(b)}
            const fn r128(a:i128,b:i128)->i128{a.rem_euclid(b)}
            const fn rs(a:isize,b:isize)->isize{a.rem_euclid(b)}
            pub const fn weeks_in_year(year:i32)->u8 {
                match year.rem_euclid(400) {4=>53,_=>52}
            }
            fn main(){
                assert_eq!(weeks_in_year(-396),53);
                assert_eq!(weeks_in_year(-400),52);
                assert_eq!(r8(-7,4),1); assert_eq!(r8(-7,-4),1);
                assert_eq!(r8(7,-4),3); assert_eq!(r8(0,-4),0);
                assert_eq!(r8(-1,i8::MIN),i8::MAX);
                assert_eq!(r16(-1,i16::MIN),i16::MAX);
                assert_eq!(r32(-1,i32::MIN),i32::MAX);
                assert_eq!(r64(-1,i64::MIN),i64::MAX);
                assert_eq!(r128(-1,i128::MIN),i128::MAX);
                assert_eq!(rs(-1,isize::MIN),isize::MAX);
                assert_eq!(r8(i8::MIN,i8::MIN),0);
                assert_eq!(r128(i128::MIN,i128::MIN),0);
                assert_eq!(r128(i128::MIN,3),1);
                assert_eq!(r128(i128::MIN,-3),1);
                assert_eq!(r128(-1,i128::MAX),i128::MAX-1);
            }"#,
                expect: Expect::Run(""),
            },
            Case {
                name: "unsigned_const_and_autoderef",
                source: r#"
            const fn r8(a:u8,b:u8)->u8{a.rem_euclid(b)}
            const fn r16(a:u16,b:u16)->u16{a.rem_euclid(b)}
            const fn r32(a:u32,b:u32)->u32{a.rem_euclid(b)}
            const fn r64(a:u64,b:u64)->u64{a.rem_euclid(b)}
            const fn r128(a:u128,b:u128)->u128{a.rem_euclid(b)}
            const fn rs(a:usize,b:usize)->usize{a.rem_euclid(b)}
            fn main(){
                assert_eq!(r8(u8::MAX,4),3);assert_eq!(r16(u16::MAX,4),3);
                assert_eq!(r32(u32::MAX,4),3);assert_eq!(r64(u64::MAX,4),3);
                assert_eq!(r128(u128::MAX,4),3);assert_eq!(rs(usize::MAX,4),3);
                let n=-7i32;let reference=&n;let nested=&reference;
                assert_eq!(nested.rem_euclid(-4),1);
            }"#,
                expect: Expect::Run(""),
            },
            Case {
                name: "single_evaluation_order",
                source: r#"
            fn operand(n:i32)->i32{println!("{}",n);n}
            fn main(){assert_eq!(operand(-7).rem_euclid(operand(-4)),1);}
            "#,
                expect: Expect::Run("-7\n-4\n"),
            },
            Case {
                name: "wrong_argument",
                source: "const fn bad(a:i32)->i32{a.rem_euclid(2u32)}fn main(){}",
                expect: Expect::Invalid("E0308"),
            },
            Case {
                name: "wrong_result",
                source: "const fn bad(a:i32)->u32{a.rem_euclid(2)}fn main(){}",
                expect: Expect::Invalid("E0308"),
            },
            Case {
                name: "wrong_arity",
                source: "const fn bad(a:i32)->i32{a.rem_euclid()}fn main(){}",
                expect: Expect::Invalid("E0061"),
            },
            Case {
                name: "same_name_runtime_inherent_is_not_const",
                source: r#"
            struct Number;
            impl Number{fn rem_euclid(self,n:i32)->i32{n}}
            const fn bad(n:Number)->i32{n.rem_euclid(4)}
            fn main(){}"#,
                expect: Expect::Invalid("E0015"),
            },
            Case {
                name: "same_name_trait_is_not_const",
                source: r#"
            trait R{fn rem_euclid(self,n:i32)->i32;}
            struct Number;
            impl R for Number{fn rem_euclid(self,n:i32)->i32{n}}
            const fn bad(n:Number)->i32{n.rem_euclid(4)}
            fn main(){}"#,
                expect: Expect::Invalid("E0015"),
            },
        ],
    );
}

#[test]
fn integer_rem_euclid_zero_and_signed_overflow_trap() {
    use std::os::unix::fs::PermissionsExt;
    use std::process::Command;
    let root = std::env::var_os("JCODE_SCRATCH_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join(format!("jr-rem-trap-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    for (index, (ty, left, right)) in [
        ("i32", "7", "0"),
        ("u32", "7", "0"),
        ("i128", "7", "0"),
        ("u128", "7", "0"),
        ("i32", "i32::MIN", "-1"),
        ("i128", "i128::MIN", "-1"),
    ]
    .iter()
    .enumerate()
    {
        let source = format!(
            "const fn rem(a:{ty},b:{ty})->{ty}{{a.rem_euclid(b)}}fn main(){{let _=rem({left},{right});}}"
        );
        let path = root.join(format!("case-{index}.rs"));
        std::fs::write(&path, &source).unwrap();
        for workers in [0, 1, 8] {
            let image = root.join(format!("image-{index}-{workers}"));
            if workers == 0 {
                let built = Command::new(stock_rustc::stock_rustc())
                    .args(["--edition=2024", "-Cpanic=abort", "--crate-name=rem"])
                    .arg(&path)
                    .arg("-o")
                    .arg(&image)
                    .output()
                    .unwrap();
                assert!(
                    built.status.success(),
                    "{}",
                    String::from_utf8_lossy(&built.stderr)
                );
            } else {
                let checked =
                    justrust_engine::compiler::compile(source.clone(), workers, true).unwrap();
                assert!(checked.executable.is_none());
                let built =
                    justrust_engine::compiler::compile(source.clone(), workers, false).unwrap();
                std::fs::write(&image, built.executable.unwrap()).unwrap();
                std::fs::set_permissions(&image, std::fs::Permissions::from_mode(0o700)).unwrap();
            }
            let ran = Command::new("sh")
                .args(["-c", "ulimit -c 0; exec \"$1\"", "rem-trap"])
                .arg(&image)
                .output()
                .unwrap();
            assert!(
                !ran.status.success(),
                "remainder trap returned successfully: {ty} {left} {right}, workers={workers}"
            );
        }
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn integer_radix_parsing_matches_stock() {
    use closure_oracle::{Case, Expect};
    parse_oracle(
        "parse-radix",
        &[
            Case {
                name: "all_widths_and_radices",
                source: r#"fn main() {
                assert_eq!(i8::from_str_radix("-80", 16).unwrap(), i8::MIN);
                assert_eq!(i16::from_str_radix("-8000", 16).unwrap(), i16::MIN);
                assert_eq!(i32::from_str_radix("-80000000", 16).unwrap(), i32::MIN);
                assert_eq!(i64::from_str_radix("-8000000000000000", 16).unwrap(), i64::MIN);
                assert_eq!(i128::from_str_radix("-80000000000000000000000000000000", 16).unwrap(), i128::MIN);
                assert_eq!(isize::from_str_radix("-8000000000000000", 16).unwrap(), isize::MIN);
                assert_eq!(u8::from_str_radix("ff", 16).unwrap(), u8::MAX);
                assert_eq!(u16::from_str_radix("fFff", 16).unwrap(), u16::MAX);
                assert_eq!(u32::from_str_radix("FFFFFFFF", 16).unwrap(), u32::MAX);
                assert_eq!(u64::from_str_radix("ffffffffffffffff", 16).unwrap(), u64::MAX);
                assert_eq!(u128::from_str_radix("ffffffffffffffffffffffffffffffff", 16).unwrap(), u128::MAX);
                assert_eq!(usize::from_str_radix("ffffffffffffffff", 16).unwrap(), usize::MAX);
                for radix in 2u32..37u32 {
                    assert_eq!(u32::from_str_radix("10", radix).unwrap(), radix);
                    assert_eq!(i32::from_str_radix("-10", radix).unwrap(), -(radix as i32));
                    assert_eq!(u128::from_str_radix("+00010", radix).unwrap(), radix as u128);
                }
                assert_eq!(u32::from_str_radix("Zz", 36).unwrap(), 1295);
                assert_eq!(i32::from_str_radix("-0", 2).unwrap(), 0);
            }"#,
                expect: Expect::Run(""),
            },
            Case {
                name: "digits_signs_and_overflow",
                source: r#"use std::num::IntErrorKind;
            fn main() {
                for text in ["", "+", "-", "0x10", "1_0", " 10", "10 ", "é", "Ｆ", "2"] {
                    println!("{:?}", u8::from_str_radix(text, 2).unwrap_err().kind());
                }
                assert_eq!(i8::from_str_radix("80",16).unwrap_err().kind(), &IntErrorKind::PosOverflow);
                assert_eq!(i8::from_str_radix("-81",16).unwrap_err().kind(), &IntErrorKind::NegOverflow);
                assert_eq!(u8::from_str_radix("100",16).unwrap_err().kind(), &IntErrorKind::PosOverflow);
                assert_eq!(u128::from_str_radix("100000000000000000000000000000000",16).unwrap_err().kind(), &IntErrorKind::PosOverflow);
                assert_eq!(i128::from_str_radix("-80000000000000000000000000000001",16).unwrap_err().kind(), &IntErrorKind::NegOverflow);
                assert_eq!(u128::from_str_radix("-0",16).unwrap_err().kind(), &IntErrorKind::InvalidDigit);
                assert_eq!(u8::from_str_radix("z",35).unwrap_err().kind(), &IntErrorKind::InvalidDigit);
            }"#,
                expect: Expect::Run(
                    "Empty\nInvalidDigit\nInvalidDigit\nInvalidDigit\nInvalidDigit\nInvalidDigit\nInvalidDigit\nInvalidDigit\nInvalidDigit\nInvalidDigit\n",
                ),
            },
            Case {
                name: "unused_wrong_radix_type",
                source: "fn bad(){let _=u32::from_str_radix(\"1\", 16u64);}fn main(){}",
                expect: Expect::Invalid("E0308"),
            },
        ],
    );
}

#[test]
fn integer_radix_out_of_range_panics_like_stock() {
    use std::io::Read;
    use std::os::unix::process::{CommandExt, ExitStatusExt};
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    unsafe extern "C" {
        fn prctl(option: std::ffi::c_int, ...) -> std::ffi::c_int;
    }
    let root = std::env::temp_dir().join(format!("jr-radix-panic-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    for radix in [0u32, 1, 37] {
        let source = root.join(format!("radix-{radix}.rs"));
        std::fs::write(
            &source,
            format!("fn main() {{ let _ = u32::from_str_radix(\"\", {radix}); }}"),
        )
        .unwrap();
        let message =
            format!("from_ascii_radix: radix must lie in the range `[2, 36]` - found {radix}");
        for workers in [0, 1, 8] {
            let image = root.join(format!("radix-{radix}-{workers}"));
            let mut command = if workers == 0 {
                let mut command = Command::new(stock_rustc::stock_rustc());
                command.args(["--edition=2024", "-Cpanic=abort", "--crate-name=radix"]);
                command
            } else {
                let mut command = Command::new(env!("CARGO_BIN_EXE_jr-rust"));
                command.args(["--workers", &workers.to_string()]);
                command
            };
            let built = command.arg(&source).arg("-o").arg(&image).output().unwrap();
            assert!(
                built.status.success(),
                "{}",
                String::from_utf8_lossy(&built.stderr)
            );
            let mut run = Command::new(&image);
            run.stdout(Stdio::null())
                .stderr(Stdio::piped())
                .env("RUST_BACKTRACE", "0");
            // SAFETY: only a libc syscall runs between fork and exec. Disabling
            // dumpability prevents a piped core service from delaying SIGABRT.
            unsafe {
                run.pre_exec(|| {
                    if prctl(4, 0usize, 0usize, 0usize, 0usize) == 0 {
                        Ok(())
                    } else {
                        Err(std::io::Error::last_os_error())
                    }
                });
            }
            let mut child = run.spawn().unwrap();
            let start = Instant::now();
            let status = loop {
                if let Some(status) = child.try_wait().unwrap() {
                    break status;
                }
                if start.elapsed() > Duration::from_secs(10) {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("radix panic timed out: {radix}, workers={workers}");
                }
                std::thread::sleep(Duration::from_millis(5));
            };
            let mut stderr = String::new();
            child
                .stderr
                .take()
                .unwrap()
                .read_to_string(&mut stderr)
                .unwrap();
            assert_eq!(status.signal(), Some(6), "{stderr}");
            assert!(stderr.contains(&message), "{stderr}");
        }
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn bool_and_char_from_str_match_stock() {
    use closure_oracle::{Case, Expect};
    parse_oracle(
        "parse-bool-char",
        &[
            Case {
                name: "boolean_exact_spelling_and_errors",
                source: r#"use std::str::{FromStr, ParseBoolError};
fn parse<T: FromStr>(s: &str) -> Result<T, T::Err> { s.parse() }
fn main() {
    assert_eq!(bool::from_str("true").unwrap(), true);
    assert_eq!("false".parse::<bool>().unwrap(), false);
    let value: bool = parse::<bool>("true").unwrap();
    assert!(value);
    assert_eq!(String::from("false").parse::<bool>().unwrap(), false);
    for text in ["", "True", "FALSE", " true", "false ", "1", "0", "truefalse"] {
        let error: ParseBoolError = text.parse::<bool>().unwrap_err();
        assert_eq!(error, error.clone());
        println!("{} {:?}", error, error);
    }
}"#,
                expect: Expect::Run(concat!(
                    "provided string was not `true` or `false` ParseBoolError\n",
                    "provided string was not `true` or `false` ParseBoolError\n",
                    "provided string was not `true` or `false` ParseBoolError\n",
                    "provided string was not `true` or `false` ParseBoolError\n",
                    "provided string was not `true` or `false` ParseBoolError\n",
                    "provided string was not `true` or `false` ParseBoolError\n",
                    "provided string was not `true` or `false` ParseBoolError\n",
                    "provided string was not `true` or `false` ParseBoolError\n",
                )),
            },
            Case {
                name: "unicode_scalar_not_byte_or_grapheme",
                source: r#"use core::str::FromStr;
use core::char::ParseCharError;
fn parse<T: FromStr>(s: &str) -> Result<T, T::Err> { s.parse() }
fn main() {
    assert_eq!(char::from_str("a").unwrap(), 'a');
    assert_eq!("é".parse::<char>().unwrap(), 'é');
    assert_eq!("🙂".parse::<char>().unwrap(), '🙂');
    let scalar: char = parse::<char>("中").unwrap();
    assert_eq!(scalar, '中');
    assert_eq!(" ".parse::<char>().unwrap(), ' ');
    assert_eq!("\0".parse::<char>().unwrap(), '\0');
    assert_eq!(String::from("é").parse::<char>().unwrap(), 'é');
    let empty: ParseCharError = "".parse::<char>().unwrap_err();
    assert_eq!(empty, empty.clone());
    println!("{} {:?}", empty, empty);
    for text in ["ab", "éa", "🙂🙂", "é"] {
        let error = text.parse::<char>().unwrap_err();
        println!("{} {:?}", error, error);
    }
}"#,
                expect: Expect::Run(concat!(
                    "cannot parse char from empty string ParseCharError { kind: EmptyString }\n",
                    "too many characters in string ParseCharError { kind: TooManyChars }\n",
                    "too many characters in string ParseCharError { kind: TooManyChars }\n",
                    "too many characters in string ParseCharError { kind: TooManyChars }\n",
                    "too many characters in string ParseCharError { kind: TooManyChars }\n",
                )),
            },
            Case {
                name: "unused_incompatible_boolean_result",
                source: "fn bad()->Result<bool,core::char::ParseCharError>{\"true\".parse::<bool>()}fn main(){}",
                expect: Expect::Invalid("E0308"),
            },
        ],
    );
}

#[test]
fn integer_from_str_explicit_dispatch() {
    parse_oracle(
        "parse-explicit",
        &[
            closure_oracle::Case {
                name: "explicit_dispatch",
                source: "fn main() { let r: Result<i32, std::num::ParseIntError> = \"42\".parse::<i32>(); match r { Ok(n) => assert_eq!(n, 42), Err(_) => panic!(\"parse failed\") } }",
                expect: closure_oracle::Expect::Run(""),
            },
            closure_oracle::Case {
                name: "contextual_result",
                source: "fn main() { let r: Result<i32, std::num::ParseIntError> = \"42\".parse(); assert_eq!(r.unwrap(), 42); }",
                expect: closure_oracle::Expect::Run(""),
            },
            closure_oracle::Case {
                name: "explicit_chain",
                source: "fn main() { let n: i32 = \"42\".parse::<i32>().unwrap(); assert_eq!(n, 42); }",
                expect: closure_oracle::Expect::Run(""),
            },
            closure_oracle::Case {
                name: "contextual_chain",
                source: "fn main() { let n: i32 = \"42\".parse().unwrap(); assert_eq!(n, 42); }",
                expect: closure_oracle::Expect::Run(""),
            },
        ],
    );
}

#[test]
fn integer_from_str_receiver_forms() {
    for (name, source) in [
        (
            "narrow_context",
            "fn main(){let n:i8=\"-128\".parse().unwrap();assert_eq!(n,i8::MIN);}",
        ),
        (
            "wide_context",
            "use std::str::FromStr;fn main(){assert_eq!(i128::from_str(\"-170141183460469231731687303715884105728\").unwrap(),i128::MIN);}",
        ),
        (
            "owned_receiver",
            "fn main(){assert_eq!(String::from(\"+00042\").parse::<i32>().unwrap(),42);}",
        ),
        (
            "borrowed_owned_receiver",
            "fn main(){let s=String::from(\"42\");assert_eq!((&s).parse::<i32>().unwrap(),42);}",
        ),
    ] {
        parse_oracle(
            name,
            &[closure_oracle::Case {
                name,
                source,
                expect: closure_oracle::Expect::Run(""),
            }],
        );
    }
    parse_oracle(
        "parse-owned-drop",
        &[
            closure_oracle::Case {
                name: "owned_deref_temporary_drop_order",
                source: r#"
use std::ops::Deref;
struct Input { text: String }
impl Deref for Input {
    type Target = str;
    fn deref(&self) -> &str { self.text.as_str() }
}
impl Drop for Input { fn drop(&mut self) { println!("drop"); } }
fn make() -> Input { println!("make"); Input { text: String::from("42") } }
fn main() {
    let value = make().parse::<i32>().unwrap();
    println!("value {}", value);
}
"#,
                expect: closure_oracle::Expect::Run("make\ndrop\nvalue 42\n"),
            },
            closure_oracle::Case {
                name: "borrowed_result_cannot_escape_owned_deref_temporary",
                source: "fn make() -> String { String::from(\"one two\") } fn main() { let mut words = make().split_whitespace(); println!(\"{}\", words.next().unwrap()); }",
                expect: closure_oracle::Expect::Invalid("E0716"),
            },
        ],
    );
}

#[test]
fn integer_from_str_matches_stock() {
    use closure_oracle::{Case, Expect};
    parse_oracle(
        "integer-from-str",
        &[
            Case {
                name: "all_integer_widths_and_context",
                source: r#"
use std::str::FromStr;
fn main() {
    let a: i8 = "-128".parse().unwrap(); assert_eq!(a, i8::MIN);
    let b: i16 = "32767".parse().unwrap(); assert_eq!(b, i16::MAX);
    let c: i32 = "-2147483648".parse().unwrap(); assert_eq!(c, i32::MIN);
    let d: i64 = "9223372036854775807".parse().unwrap(); assert_eq!(d, i64::MAX);
    let e: isize = "-9223372036854775808".parse().unwrap(); assert_eq!(e, isize::MIN);
    assert_eq!("255".parse::<u8>().unwrap(), u8::MAX);
    assert_eq!("65535".parse::<u16>().unwrap(), u16::MAX);
    assert_eq!("4294967295".parse::<u32>().unwrap(), u32::MAX);
    assert_eq!("18446744073709551615".parse::<u64>().unwrap(), u64::MAX);
    assert_eq!("18446744073709551615".parse::<usize>().unwrap(), usize::MAX);
    assert_eq!(i128::from_str("-170141183460469231731687303715884105728").unwrap(), i128::MIN);
    assert_eq!("170141183460469231731687303715884105727".parse::<i128>().unwrap(), i128::MAX);
    assert_eq!("340282366920938463463374607431768211455".parse::<u128>().unwrap(), u128::MAX);
    assert_eq!(String::from("+00042").parse::<i32>().unwrap(), 42);
    assert_eq!("-0".parse::<i8>().unwrap(), 0);
    assert_eq!("+0".parse::<u128>().unwrap(), 0);
}
"#,
                expect: Expect::Run(""),
            },
            Case {
                name: "error_kinds_and_formatting",
                source: r#"
use std::num::{IntErrorKind, ParseIntError};
fn main() {
    let empty: ParseIntError = "".parse::<i32>().unwrap_err();
    assert_eq!(empty.kind(), &IntErrorKind::Empty);
    println!("{} {:?}", empty, empty);
    let digit = " 42".parse::<i32>().unwrap_err();
    assert_eq!(digit.kind(), &IntErrorKind::InvalidDigit);
    println!("{} {:?}", digit, digit);
    let positive = "128".parse::<i8>().unwrap_err();
    assert_eq!(positive.kind(), &IntErrorKind::PosOverflow);
    println!("{} {:?}", positive, positive);
    let negative = "-129".parse::<i8>().unwrap_err();
    assert_eq!(negative.kind(), &IntErrorKind::NegOverflow);
    println!("{} {:?}", negative, negative);
    assert!("-0".parse::<u8>().is_err());
    assert!("+".parse::<u128>().is_err());
    assert!("-".parse::<i128>().is_err());
    assert!("1_0".parse::<u64>().is_err());
    assert!("42 ".parse::<i32>().is_err());
    assert!("１２".parse::<i32>().is_err());
    assert!("1\0".parse::<u32>().is_err());
    assert!("170141183460469231731687303715884105728".parse::<i128>().is_err());
    assert!("-170141183460469231731687303715884105729".parse::<i128>().is_err());
    assert!("340282366920938463463374607431768211456".parse::<u128>().is_err());
}
"#,
                expect: Expect::Run(
                    "cannot parse integer from empty string ParseIntError { kind: Empty }\ninvalid digit found in string ParseIntError { kind: InvalidDigit }\nnumber too large to fit in target type ParseIntError { kind: PosOverflow }\nnumber too small to fit in target type ParseIntError { kind: NegOverflow }\n",
                ),
            },
            Case {
                name: "custom_from_str_and_core_path",
                source: r#"
use core::str::FromStr;
struct Value { n: i32 }
impl FromStr for Value {
    type Err = core::num::ParseIntError;
    fn from_str(s: &str) -> Result<Value, Self::Err> {
        match s.parse::<i32>() {
            Ok(n) => Ok(Value { n }),
            Err(e) => Err(e),
        }
    }
}
fn parse<T: FromStr>(s: &str) -> Result<T, T::Err> { s.parse() }
fn main() {
    let value = parse::<Value>("42").ok().unwrap();
    assert_eq!(value.n, 42);
    assert!(parse::<Value>("no").is_err());
}
"#,
                expect: Expect::Run(""),
            },
            Case {
                name: "unused_missing_from_str",
                source: "struct Value; fn unused() { let _v = \"1\".parse::<Value>(); } fn main() {}",
                expect: Expect::Invalid("E0277"),
            },
            Case {
                name: "unused_wrong_error_type",
                source: "fn unused() { let _v: Result<i32, u8> = \"1\".parse::<i32>(); } fn main() {}",
                expect: Expect::Invalid("E0308"),
            },
            Case {
                name: "parser_static_error_does_not_borrow_input",
                source: r#"use std::str::FromStr;
struct Value;
impl FromStr for Value {
    type Err = &'static str;
    fn from_str(_s: &str) -> Result<Value, Self::Err> { Err("invalid") }
}
fn main() {
    let error;
    { let text = String::from("42"); error = text.parse::<Value>().err().unwrap(); }
    println!("{}", error);
}"#,
                expect: Expect::Run("invalid\n"),
            },
            Case {
                name: "unused_nested_static_cannot_borrow_input",
                source: "fn bad(s: &str) -> Result<i32, &'static str> { Err(s) } fn main() {}",
                expect: Expect::Invalid("lifetime may not live long enough"),
            },
            Case {
                name: "unused_parser_cannot_return_input_as_static_error",
                source: r#"use std::str::FromStr;
struct Value;
impl FromStr for Value {
    type Err = &'static str;
    fn from_str(s: &str) -> Result<Value, Self::Err> { Err(s) }
}
fn main() {}"#,
                expect: Expect::Invalid("lifetime may not live long enough"),
            },
            Case {
                name: "method_local_projection_preserves_exact_binder",
                source: r#"
trait Has { type Out; }
struct Item;
impl Has for Item { type Out = i32; }
trait Forward { fn forward<T: Has>(&self, value: T::Out) -> T::Out; }
struct Sender;
impl Forward for Sender {
    fn forward<T: Has>(&self, value: T::Out) -> T::Out { value }
}
fn main() { assert_eq!(Sender.forward::<Item>(42), 42); }
"#,
                expect: Expect::Run(""),
            },
            Case {
                name: "unused_generic_projected_method_wrong_body",
                source: r#"
trait Has { type Out; }
trait Forward { fn forward<T: Has>(&self, value: T::Out) -> T::Out; }
struct Sender;
impl Forward for Sender {
    fn forward<T: Has>(&self, value: T::Out) -> T::Out { 42i32 }
}
fn main() {}
"#,
                expect: Expect::Invalid("E0308"),
            },
        ],
    );
}

#[test]
fn integer_from_str_boundary_matrix() {
    let mut inputs = vec![
        "".to_owned(),
        "+".to_owned(),
        "-".to_owned(),
        "+0".to_owned(),
        "-0".to_owned(),
        "00042".to_owned(),
        " 1".to_owned(),
        "1 ".to_owned(),
        "\t1".to_owned(),
        "1\n".to_owned(),
        "1_0".to_owned(),
        "0x10".to_owned(),
        "++1".to_owned(),
        "+-1".to_owned(),
        "١".to_owned(),
        "１２".to_owned(),
        "1\0".to_owned(),
        "127".to_owned(),
        "128".to_owned(),
        "-128".to_owned(),
        "-129".to_owned(),
        "255".to_owned(),
        "256".to_owned(),
        "32767".to_owned(),
        "32768".to_owned(),
        "-32768".to_owned(),
        "-32769".to_owned(),
        "65535".to_owned(),
        "65536".to_owned(),
        "2147483647".to_owned(),
        "2147483648".to_owned(),
        "-2147483648".to_owned(),
        "-2147483649".to_owned(),
        "4294967295".to_owned(),
        "4294967296".to_owned(),
        "9223372036854775807".to_owned(),
        "9223372036854775808".to_owned(),
        "-9223372036854775808".to_owned(),
        "-9223372036854775809".to_owned(),
        "18446744073709551615".to_owned(),
        "18446744073709551616".to_owned(),
        "170141183460469231731687303715884105727".to_owned(),
        "170141183460469231731687303715884105728".to_owned(),
        "-170141183460469231731687303715884105728".to_owned(),
        "-170141183460469231731687303715884105729".to_owned(),
        "340282366920938463463374607431768211455".to_owned(),
        "340282366920938463463374607431768211456".to_owned(),
        "99999999999999999999999999999999999999999999x".to_owned(),
        "-99999999999999999999999999999999999999999999x".to_owned(),
    ];
    inputs.push(format!("{}42", "0".repeat(1024)));
    let mut source = String::new();
    let mut calls = String::from("fn main() {");
    let mut expected = String::new();
    macro_rules! add_type {
        ($ty:ty) => {{
            let name = stringify!($ty);
            source.push_str(&format!("fn cases_{name}() {{ for text in {inputs:?} {{ println!(\"{{:?}}\", text.parse::<{name}>()); }} }}\n"));
            calls.push_str(&format!("cases_{name}();"));
            for input in &inputs {
                expected.push_str(&format!("{:?}\n", input.parse::<$ty>()));
            }
        }};
    }
    add_type!(i8);
    add_type!(i16);
    add_type!(i32);
    add_type!(i64);
    add_type!(i128);
    add_type!(isize);
    add_type!(u8);
    add_type!(u16);
    add_type!(u32);
    add_type!(u64);
    add_type!(u128);
    add_type!(usize);
    calls.push('}');
    source.push_str(&calls);
    parse_oracle(
        "parse-boundaries",
        &[closure_oracle::Case {
            name: "all_widths_syntax_and_overflow",
            source: Box::leak(source.into_boxed_str()),
            expect: closure_oracle::Expect::Run(Box::leak(expected.into_boxed_str())),
        }],
    );
}

#[test]
fn primitive_methods_match_stock() {
    assert!(
        std::process::Command::new("rustc")
            .arg("-vV")
            .output()
            .is_ok_and(|o| o.status.success()),
        "primitive differential tests require local stock rustc"
    );
    oracle::check(
        "primitive-methods",
        &[
            Case {
                name: "signed",
                source: r#"fn main() {
            assert_eq!((-7i32).abs(), 7); assert_eq!((-7i32).signum(), -1);
            assert_eq!(0i32.signum(), 0); assert_eq!(7i32.signum(), 1);
            assert!((-1i64).is_negative()); assert!(1i64.is_positive());
            assert_eq!(3i16.pow(4), 81); assert_eq!(127i8.pow(1), 127); assert_eq!(0i8.pow(0), 1);
            assert_eq!(7i32.min(3), 3); assert_eq!(7i32.max(3), 7);
        }"#,
                expect: Expect::Run(""),
            },
            Case {
                name: "wrapping",
                source: r#"fn main() {
            assert_eq!(127i8.wrapping_add(1), -128); assert_eq!((-128i8).wrapping_sub(1), 127);
            assert_eq!(100i8.wrapping_mul(3), 44); assert_eq!((-128i8).wrapping_neg(), -128);
            assert_eq!(255u8.wrapping_add(2), 1); assert_eq!(0u8.wrapping_sub(1), 255);
            assert_eq!(255u8.wrapping_mul(255), 1);
            assert_eq!(127i8.saturating_add(1), 127); assert_eq!((-128i8).saturating_sub(1), -128);
            assert_eq!(255u8.saturating_add(1), 255); assert_eq!(0u8.saturating_sub(1), 0);
            assert_eq!(18446744073709551615u64.wrapping_mul(18446744073709551615u64), 1);
        }"#,
                expect: Expect::Run(""),
            },
            Case {
                name: "bits",
                source: r#"fn main() {
            assert_eq!((-1i8).count_ones(), 8); assert_eq!(0u16.count_zeros(), 16);
            assert_eq!(0u128.trailing_zeros(), 128); assert_eq!(8u32.trailing_zeros(), 3);
            assert_eq!((-128i8).count_ones(), 1); assert!(128u8.is_power_of_two());
            assert!(!0u8.is_power_of_two()); assert!(!3u8.is_power_of_two());
        }"#,
                expect: Expect::Run(""),
            },
            Case {
                name: "characters",
                source: r#"fn main() {
            assert!('A'.is_ascii()); assert!('9'.is_ascii_digit()); assert!('f'.is_ascii_hexdigit());
            assert!('Z'.is_ascii_alphabetic()); assert!('3'.is_ascii_alphanumeric());
            assert!('A'.is_ascii_uppercase()); assert!('z'.is_ascii_lowercase());
            assert!('\t'.is_ascii_whitespace()); assert!(!'\u{b}'.is_ascii_whitespace());
            assert!('!'.is_ascii_punctuation()); assert!('~'.is_ascii_graphic());
            assert!('\u{7f}'.is_ascii_control()); assert!('\u{85}'.is_control());
            assert!('\u{2003}'.is_whitespace()); assert!(!'\u{200b}'.is_whitespace());
            assert!(!'é'.is_ascii()); assert_eq!('a'.to_ascii_uppercase(), 'A');
            assert_eq!('Z'.to_ascii_lowercase(), 'z'); assert_eq!('é'.to_ascii_uppercase(), 'é');
        }"#,
                expect: Expect::Run(""),
            },
            Case {
                name: "bytes_ascii",
                source: r#"fn main() {
    let bytes = [b'7', b'a', b'Z', b' ', 0x0bu8, 0x0c, b'!', b'~', 0x7f, 0x80, b'f', b'G', b'\n', 0];
    for b in bytes {
        println!("{} {}{}{}{}{}{}{}{}{}{}{} {} {}", b,
            b.is_ascii() as u8, b.is_ascii_alphabetic() as u8, b.is_ascii_digit() as u8,
            b.is_ascii_alphanumeric() as u8, b.is_ascii_whitespace() as u8, b.is_ascii_punctuation() as u8,
            b.is_ascii_uppercase() as u8, b.is_ascii_lowercase() as u8, b.is_ascii_hexdigit() as u8,
            b.is_ascii_graphic() as u8, b.is_ascii_control() as u8,
            b.to_ascii_uppercase(), b.to_ascii_lowercase());
    }
    println!("{} {} {}", b'a'.eq_ignore_ascii_case(&b'A'), b'1'.eq_ignore_ascii_case(&b'1'), b'a'.eq_ignore_ascii_case(&b'b'));
    let s: &[u8] = b"x-12";
    let r: &u8 = &s[2];
    println!("{} {}", r.is_ascii_digit(), s.iter().filter(|c| c.is_ascii_digit()).count());
}"#,
                expect: Expect::Run(
                    "55 10110000110 55 55\n97 11010001110 65 97\n90 11010010010 90 122\n32 10001000000 32 32\n11 10000000001 11 11\n12 10001000001 12 12\n33 10000100010 33 33\n126 10000100010 126 126\n127 10000000001 127 127\n128 00000000000 128 128\n102 11010001110 70 102\n71 11010010010 71 103\n10 10001000001 10 10\n0 10000000001 0 0\ntrue true false\ntrue 2\n",
                ),
            },
            Case {
                name: "wider_int_ascii_method_unused",
                source: r#"#[allow(dead_code)]
fn f() -> bool { 300u16.is_ascii_digit() }
fn main() {}"#,
                expect: Expect::Invalid("E0599"),
            },
            Case {
                name: "floats",
                source: r#"fn main() {
            assert_eq!(9.0f64.sqrt(), 3.0); assert_eq!((-4.0f32).abs(), 4.0);
            assert!(0.0f64.is_finite()); assert!((-0.0f32).is_sign_negative());
            assert!(0.0f64.is_sign_positive()); assert!(!(1.0f64).is_nan());
            let nan: f64 = 0.0f64 / 0.0f64; let inf: f64 = 1.0f64 / 0.0f64;
            assert!(nan.is_nan()); assert!(inf.is_infinite()); assert!(!inf.is_finite());
            assert_eq!(nan.min(2.0), 2.0); assert_eq!(2.0f64.min(nan), 2.0);
            assert_eq!(nan.max(2.0), 2.0); assert_eq!(2.0f64.max(nan), 2.0);
            assert_eq!(2.0f32.min(3.0), 2.0); assert_eq!(2.0f32.max(3.0), 3.0);
        }"#,
                expect: Expect::Run(""),
            },
            Case {
                name: "autoderef",
                source: r#"fn main() {
            let n: i32 = -5; let r: &i32 = &n; let rr: &&i32 = &r;
            assert_eq!(rr.abs(), 5); let c: char = 'a'; let cr: &char = &c;
            assert!(cr.is_ascii_lowercase()); let mut f: f64 = 4.0;
            let fr: &mut f64 = &mut f; assert_eq!(fr.sqrt(), 2.0);
        }"#,
                expect: Expect::Run(""),
            },
            Case {
                name: "evaluation_order",
                source: r#"fn main() {
            let mut n: i32 = 5;
            let result: i32 = n.min({ n = 1; 3 });
            assert_eq!(result, 3); assert_eq!(n, 1);
            let mut order: i32 = 0;
            let value: i32 = { order = order * 10 + 1; 7i32 }.wrapping_add({ order = order * 10 + 2; 3 });
            assert_eq!(value, 10); assert_eq!(order, 12);
        }"#,
                expect: Expect::Run(""),
            },
            Case {
                name: "wide_and_float_edges",
                source: r#"fn main() {
            assert_eq!((-170141183460469231731687303715884105727i128).abs(), 170141183460469231731687303715884105727i128);
            assert_eq!((-1i128).count_ones(), 128);
            assert_eq!(340282366920938463463374607431768211455u128.count_zeros(), 0);
            assert_eq!(2u128.pow(100), 1267650600228229401496703205376u128);
            assert_eq!((-9223372036854775808i64).saturating_add(-1), -9223372036854775808i64);
            assert_eq!(9223372036854775807i64.saturating_sub(-1), 9223372036854775807i64);
            assert_eq!(255u8.saturating_sub(1), 254);
            assert_eq!((-128i8).saturating_add(-1), -128);
            assert_eq!(127i8.saturating_sub(-1), 127);
            assert_eq!((-128i8).saturating_add(127), -1);
            assert_eq!(255u8.saturating_add(0), 255);
            assert_eq!((-0.0f64).min(0.0), 0.0); assert_eq!(0.0f64.min(-0.0), 0.0);
            assert_eq!((-0.0f64).max(0.0), 0.0); assert_eq!(0.0f64.max(-0.0), 0.0);
            assert_eq!((-0.0f32).min(0.0), 0.0); assert_eq!(0.0f32.min(-0.0), 0.0);
            assert_eq!((-0.0f32).max(0.0), 0.0); assert_eq!(0.0f32.max(-0.0), 0.0);
            let nan: f32 = 0.0f32 / 0.0f32;
            assert!(nan.min(nan).is_nan()); assert_eq!(2.0f32.min(nan), 2.0);
            assert_eq!(2.0f32.max(nan), 2.0); assert!((-1.0f32).sqrt().is_nan());
            assert!(!nan.is_finite()); assert!(!nan.is_infinite());
            assert!(!(-0.0f64).abs().is_sign_negative());
        }"#,
                expect: Expect::Run(""),
            },
            Case {
                name: "wrong_argument_type",
                source: "fn main() { let x: i32 = 2; x.wrapping_add(1u32); }",
                expect: Expect::Invalid("E0308"),
            },
            Case {
                name: "wrong_pow_exponent",
                source: "fn main() { 2i32.pow(3i32); }",
                expect: Expect::Invalid("E0308"),
            },
            Case {
                name: "extra_argument",
                source: "fn main() { 'a'.is_ascii(1); }",
                expect: Expect::Invalid("E0061"),
            },
            Case {
                name: "missing_argument",
                source: "fn main() { 1i32.min(); }",
                expect: Expect::Invalid("E0061"),
            },
            Case {
                name: "wrong_result",
                source: "fn main() { let x: bool = 1u8.count_ones(); }",
                expect: Expect::Invalid("E0308"),
            },
            Case {
                name: "unused_invalid",
                source: "fn unused() { 1.0f32.min(2.0f64); } fn main() {}",
                expect: Expect::Invalid("E0308"),
            },
        ],
    );
}

#[test]
fn primitive_result_inference_matches_stock_at_both_worker_counts() {
    use std::{fs, process::Command};
    let root = std::env::var_os("JCODE_SCRATCH_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join(format!("jr-primitive-inference-{}", std::process::id()));
    fs::create_dir(&root).unwrap();
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(root.clone());
    fn capture(
        command: &mut Command,
        context: &str,
        root: &std::path::Path,
    ) -> std::process::Output {
        use std::time::{Duration, Instant};
        let stdout = root.join("command.stdout");
        let stderr = root.join("command.stderr");
        command
            .stdout(fs::File::create(&stdout).unwrap())
            .stderr(fs::File::create(&stderr).unwrap());
        let mut child = command.spawn().unwrap();
        let start = Instant::now();
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if start.elapsed() > Duration::from_secs(30) {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("timed out {context}: {command:?}");
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        std::process::Output {
            status,
            stdout: fs::read(stdout).unwrap(),
            stderr: fs::read(stderr).unwrap(),
        }
    }
    let cases = [
        (
            "once",
            r#"
            fn next(calls: &mut u32) -> u128 {
                *calls += 1u32;
                18446744073709551616u128
            }
            fn main() {
                let mut calls = 0u32;
                let count = next(&mut calls).leading_zeros();
                let ones = next(&mut calls).count_ones();
                assert_eq!(calls, 2u32);
                assert_eq!(count, 63u32);
                assert_eq!(ones, 1u32);
                println!("ok");
            }"#,
            None,
        ),
        (
            "primitive_signatures",
            r#"
            fn main() {
                let absolute = (-4i32).abs();
                let character = 'a'.to_ascii_uppercase();
                let root = 9.0f64.sqrt();
                let power = 2i32.pow(3);
                let zeros = 8u32.trailing_zeros();
                let nested = 1u64.leading_zeros().count_ones();
                assert_eq!(absolute, 4i32);
                assert_eq!(character, 'A');
                assert_eq!(root, 3.0f64);
                assert_eq!(power, 8i32);
                assert_eq!(zeros, 3u32);
                assert_eq!(nested, 6u32);
                println!("ok");
            }"#,
            None,
        ),
        (
            "ordinary_method",
            r#"
            struct S { value: bool }
            impl S { fn leading_zeros(self) -> bool { self.value } }
            fn main() {
                let result = S { value: true }.leading_zeros();
                assert!(result);
                println!("ok");
            }"#,
            None,
        ),
        (
            "ordinary_scalar_trait_method",
            r#"
            trait UserCount { fn leading_zeros(self) -> bool; }
            impl UserCount for f32 { fn leading_zeros(self) -> bool { self == 1.0f32 } }
            fn main() {
                let result = 1.0f32.leading_zeros();
                assert!(result);
                println!("ok");
            }"#,
            None,
        ),
        (
            "wrong_result",
            "fn main(){let count: bool = 1u128.leading_zeros();}",
            Some("E0308"),
        ),
        (
            "wrong_argument",
            "fn main(){let result = 1i32.wrapping_add(1u32);}",
            Some("E0308"),
        ),
        (
            "wrong_arity",
            "fn main(){let count = 1u128.leading_zeros(1u32);}",
            Some("E0061"),
        ),
        (
            "wrong_type_arguments",
            "fn main(){let count = 1u128.leading_zeros::<u8>();}",
            Some("E0107"),
        ),
        (
            "noninteger",
            "fn main(){let count = 1.0f32.leading_zeros();}",
            Some("E0599"),
        ),
        (
            "unused_wrong_result",
            "fn unused(){let count: bool = 1u128.leading_zeros();} fn main(){}",
            Some("E0308"),
        ),
    ];
    std::thread::scope(|scope| {
        let mut jobs = Vec::new();
        for (name, source, invalid) in cases {
            let root = root.join(name);
            jobs.push(scope.spawn(move || {
                fs::create_dir(&root).unwrap();
                let source_path = root.join(format!("{name}.rs"));
                fs::write(&source_path, source).unwrap();
                for workers in [None, Some(1), Some(8)] {
                    for check in [true, false] {
                        let image = root.join(format!("{name}-{workers:?}-{check}"));
                        let mut command = if let Some(workers) = workers {
                            let mut command = Command::new(env!("CARGO_BIN_EXE_jr-rust"));
                            command.args(["--workers", &workers.to_string()]);
                            if check {
                                command.arg("--check");
                            }
                            command
                        } else {
                            let mut command = Command::new(stock_rustc::stock_rustc());
                            command.args(["--edition=2024", "--crate-name=oracle", "-Awarnings"]);
                            if check {
                                command.arg("--emit=metadata");
                            }
                            command
                        };
                        command.arg(&source_path);
                        if workers.is_none() || !check {
                            command.arg("-o").arg(&image);
                        }
                        let context = format!("{name} workers={workers:?} check={check}");
                        let output = capture(&mut command, &context, &root);
                        let error = String::from_utf8_lossy(&output.stderr);
                        if let Some(code) = invalid {
                            assert!(
                                !output.status.success(),
                                "{name} {workers:?}: accepted invalid source"
                            );
                            assert!(
                                output.status.code().is_some(),
                                "{name} {workers:?}: {error}"
                            );
                            assert!(!image.exists(), "{name}: invalid input published output");
                            if workers.is_none() {
                                assert!(error.contains(code), "{name}: {error}");
                            } else {
                                for forbidden in
                                    ["panicked at", "resource limit", "cannot progress"]
                                {
                                    assert!(!error.contains(forbidden), "{name}: {error}");
                                }
                            }
                        } else {
                            assert!(
                                output.status.success(),
                                "{name} {workers:?} check={check}: {error}"
                            );
                            if !check {
                                let ran = capture(&mut Command::new(&image), &context, &root);
                                assert!(ran.status.success(), "{name} {workers:?}: {ran:?}");
                                assert_eq!(ran.stdout, b"ok\n", "{name} {workers:?}");
                                assert!(ran.stderr.is_empty(), "{name} {workers:?}: {ran:?}");
                            } else if workers.is_some() {
                                assert!(!image.exists(), "CHECK published executable");
                            }
                        }
                    }
                }
            }));
        }
        for job in jobs {
            job.join().unwrap();
        }
    });
}

#[test]
fn checked_primitive_overflow_matches_stock() {
    use std::{fs, process::Command};
    let root = std::env::var_os("JCODE_SCRATCH_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join(format!("jr-primitive-overflow-{}", std::process::id()));
    fs::create_dir(&root).unwrap();
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(root.clone());
    for (name, source) in [
        (
            "abs_min",
            "fn main() { let n: i8 = -128; let result: i8 = n.abs(); assert_eq!(result, 0); }",
        ),
        (
            "pow_overflow",
            "fn main() { let n: i8 = 12; let result: i8 = n.pow(2); assert_eq!(result, 0); }",
        ),
    ] {
        let path = root.join("main.rs");
        fs::write(&path, source).unwrap();
        for (compiler, stock) in [("rustc", true), (env!("CARGO_BIN_EXE_jr-rust"), false)] {
            let image = root.join(format!("{name}-{}", if stock { "stock" } else { "native" }));
            let mut command = Command::new(compiler);
            if stock {
                command.env_remove("RUSTC_BOOTSTRAP").args([
                    "--edition=2024",
                    "-Cpanic=abort",
                    "-Coverflow-checks=yes",
                    "--crate-name=oracle",
                ]);
            }
            let output = command.arg(&path).arg("-o").arg(&image).output().unwrap();
            assert!(
                output.status.success(),
                "{name} {compiler}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let ran = Command::new(&image).output().unwrap();
            assert!(!ran.status.success(), "{name} {compiler} did not trap");
            assert!(
                !String::from_utf8_lossy(&ran.stderr).contains("assertion failed"),
                "{name} {compiler} reached the assertion instead of trapping overflow"
            );
        }
    }
}

#[test]
fn float_from_str_matches_stock() {
    use closure_oracle::{Case, Expect};
    parse_oracle(
        "parse-float",
        &[
            Case {
                name: "correctly_rounded_edges_and_errors",
                source: r#"fn main(){
    for s in ["2.5","1.","0.1",".5","1e5","1E+5","-0","+3","inf","-Infinity","NaN","nan","1e400","1e-400","2.2250738585072014e-308","4.9e-324","2.4703282292062327e-324","2.4703282292062328e-324","0.30000000000000004","123456789012345678901234567890","9007199254740993","", ".", "e5", "1e", "+", " 1", "1 ", "0x10", "1_0", "1e+", "--1", "infinit", "-nan", "+inf", "INFINITY", "1e99999999999999999999", "-1e-99999999999999999999", "00000.000001e6", "1.7976931348623157e308", "1.7976931348623158e308", "1.7976931348623159e308", "2.2250738585072011e-308"] {
        let r: Result<f64, std::num::ParseFloatError> = s.parse::<f64>();
        match r {
            Ok(v) => { let b = v.to_bits(); println!("{:?} -> {}", s, b); }
            Err(e) => println!("{:?} -> err {} {:?}", s, e, e),
        }
    }
    for s in ["3.4028235e38","3.4028236e38","1e-46","1.4e-45","0.1","16777217","-nan"] {
        let v: f32 = s.parse().unwrap();
        let b = v.to_bits();
        println!("{:?} -> {}", s, b);
    }
}
"#,
                expect: Expect::Run(
                    r#""2.5" -> 4612811918334230528
"1." -> 4607182418800017408
"0.1" -> 4591870180066957722
".5" -> 4602678819172646912
"1e5" -> 4681608360884174848
"1E+5" -> 4681608360884174848
"-0" -> 9223372036854775808
"+3" -> 4613937818241073152
"inf" -> 9218868437227405312
"-Infinity" -> 18442240474082181120
"NaN" -> 9221120237041090560
"nan" -> 9221120237041090560
"1e400" -> 9218868437227405312
"1e-400" -> 0
"2.2250738585072014e-308" -> 4503599627370496
"4.9e-324" -> 1
"2.4703282292062327e-324" -> 0
"2.4703282292062328e-324" -> 1
"0.30000000000000004" -> 4599075939470750516
"123456789012345678901234567890" -> 5042042089369253694
"9007199254740993" -> 4845873199050653696
"" -> err cannot parse float from empty string ParseFloatError { kind: Empty }
"." -> err invalid float literal ParseFloatError { kind: Invalid }
"e5" -> err invalid float literal ParseFloatError { kind: Invalid }
"1e" -> err invalid float literal ParseFloatError { kind: Invalid }
"+" -> err invalid float literal ParseFloatError { kind: Invalid }
" 1" -> err invalid float literal ParseFloatError { kind: Invalid }
"1 " -> err invalid float literal ParseFloatError { kind: Invalid }
"0x10" -> err invalid float literal ParseFloatError { kind: Invalid }
"1_0" -> err invalid float literal ParseFloatError { kind: Invalid }
"1e+" -> err invalid float literal ParseFloatError { kind: Invalid }
"--1" -> err invalid float literal ParseFloatError { kind: Invalid }
"infinit" -> err invalid float literal ParseFloatError { kind: Invalid }
"-nan" -> 18444492273895866368
"+inf" -> 9218868437227405312
"INFINITY" -> 9218868437227405312
"1e99999999999999999999" -> 9218868437227405312
"-1e-99999999999999999999" -> 9223372036854775808
"00000.000001e6" -> 4607182418800017408
"1.7976931348623157e308" -> 9218868437227405311
"1.7976931348623158e308" -> 9218868437227405311
"1.7976931348623159e308" -> 9218868437227405312
"2.2250738585072011e-308" -> 4503599627370495
"3.4028235e38" -> 2139095039
"3.4028236e38" -> 2139095040
"1e-46" -> 0
"1.4e-45" -> 1
"0.1" -> 1036831949
"16777217" -> 1266679808
"-nan" -> 4290772992
"#,
                ),
            },
            Case {
                name: "corpus18_parse_with_map_err",
                source: "fn main(){let s=String::from(\"2.5\");let n:f64=s.parse().map_err(|_|format!(\"bad {}\",s)).unwrap();let b:Result<f64,String>=\"x\".parse::<f64>().map_err(|e|format!(\"{}\",e));println!(\"{} {:?}\",n,b);}",
                expect: Expect::Run("2.5 Err(\"invalid float literal\")\n"),
            },
            Case {
                name: "float_error_is_not_int_error",
                source: "fn bad(){let _e:std::num::ParseIntError=\"1.5\".parse::<f64>().unwrap_err();}fn main(){}",
                expect: Expect::Invalid("E0308"),
            },
        ],
    );
}

/// A closure parameter typed by a projection (`<T as FromStr>::Err`) whose
/// normal form is a source struct, with `T` known only from the annotation.
#[test]
fn closure_parameter_projection_to_struct_matches_stock() {
    use closure_oracle::{Case, Expect};
    parse_oracle(
        "closure-projection-struct",
        &[
            Case {
                name: "parse_map_err_question_mark",
                source: r#"fn f(s: String) -> Result<f64, String> {
    let n: f64 = s.parse().map_err(|_| format!("bad {}", s))?;
    Ok(n)
}
fn g(s: &str) -> Result<bool, u8> { let b: bool = s.parse().map_err(|_| 7u8)?; Ok(b) }
fn main() {
    println!("{:?} {:?}", f("2.5".to_string()), f("x".to_string()));
    println!("{:?} {:?}", g("true"), g("nope"));
    let r: Result<i32, u8> = "12".parse().map_err(|_| 0u8);
    println!("{:?}", r);
}"#,
                expect: Expect::Run("Ok(2.5) Err(\"bad x\")\nOk(true) Err(7)\nOk(12)\n"),
            },
            Case {
                name: "user_trait_struct_projection",
                source: r#"#[derive(Debug)]
struct MyErr;
trait Q: Sized { type Err; fn q(s: &str) -> Result<Self, Self::Err>; }
impl Q for i32 { type Err = MyErr; fn q(s: &str) -> Result<i32, MyErr> { if s.is_empty() { Err(MyErr) } else { Ok(7) } } }
fn get<T: Q>(s: &str) -> Result<T, T::Err> { T::q(s) }
fn main() {
    let a: Result<i32, u8> = get("x").map_err(|_| 0u8);
    let b: Result<i32, u8> = get("").map_err(|_e| 9u8);
    println!("{:?} {:?}", a, b);
}"#,
                expect: Expect::Run("Ok(7) Err(9)\n"),
            },
            Case {
                name: "closure_param_used_as_wrong_type",
                source: r#"fn bad(s: &str) -> Result<i32, u8> { let n: i32 = s.parse().map_err(|e: u8| e)?; Ok(n) } fn main() {}"#,
                expect: Expect::Invalid("E0271"),
            },
            Case {
                name: "unused_adapter_context_cannot_override_original_input",
                source: r#"fn get<T>(value: T) -> Result<T, bool> { Ok(value) }
fn unused() { let result: Result<i32, u8> = get(true).map_err(|_| 0u8); }
fn main() {}"#,
                expect: Expect::Invalid("E0308"),
            },
            Case {
                name: "unused_adapter_mapped_result_must_match_authored_context",
                source: r#"fn get<T>(value: T) -> Result<T, bool> { Ok(value) }
fn unused() { let result: Result<i32, u8> = get(7i32).map_err(|_| true); }
fn main() {}"#,
                expect: Expect::Invalid("E0308"),
            },
        ],
    );
}

/// `T op &T` with the reference operand still open when the operator is
/// queued (enumerate tuple patterns), and `T op= &T`.
#[test]
fn scalar_operators_with_reference_operands_match_stock() {
    use closure_oracle::{Case, Expect};
    parse_oracle(
        "scalar-ref-operands",
        &[
            Case {
                name: "enumerate_mul_and_ref_compound_assign",
                source: r#"fn main() {
    let xs: Vec<u32> = vec![1, 9, 25];
    let mut total = 0u32;
    let mut t2 = 0u32;
    for (i, x) in xs.iter().enumerate() {
        total += i as u32 * x;
        t2 += x;
    }
    println!("{} {}", total, t2);
}"#,
                expect: Expect::Run("59 35\n"),
            },
            Case {
                name: "ref_compound_assign_wrong_type",
                source: r#"fn main() { let a = 1u8; let mut t = 0u32; t += &a; println!("{}", t); }"#,
                expect: Expect::Invalid("E0308"),
            },
        ],
    );
}

#[test]
fn opaque_physical_cleanup_unused_expression_statement_order() {
    opaque_cleanup_order_case(
        "opaque-cleanup-unused",
        r#"
fn main() { make(8); println!("between"); make(9); println!("end"); }
"#,
        "make 8\ndrop 8\nbetween\nmake 9\ndrop 9\nend\n",
    );
}
#[test]
fn opaque_physical_cleanup_borrowed_temporary_lifetime() {
    opaque_cleanup_order_case(
        "opaque-cleanup-borrowed",
        r#"
fn main() { let borrowed = &make(10); println!("value {}", borrowed.id()); println!("end"); }
"#,
        "make 10\nvalue 10\nend\ndrop 10\n",
    );
}
#[test]
fn opaque_physical_cleanup_rejects_unused_borrow_escape() {
    parse_oracle(
        "opaque-cleanup-escape",
        &[closure_oracle::Case {
            name: "unused_opaque_borrow_outlives_owner",
            source: "trait Tag{fn id(&self)->i32;}struct Token{n:i32}impl Tag for Token{fn id(&self)->i32{self.n}}impl Drop for Token{fn drop(&mut self){}}fn make()->impl Tag{Token{n:11}}fn unused(){let output;{let value=make();output=&value;}let _n=output.id();}fn main(){}",
            expect: closure_oracle::Expect::Invalid("E0597"),
        }],
    );
}

fn opaque_cleanup_order_case(name: &'static str, body: &str, expected: &'static str) {
    let source = format!(
        r#"
trait Tag {{ fn id(&self) -> i32; }}
struct Token {{ n: i32 }}
impl Tag for Token {{ fn id(&self) -> i32 {{ self.n }} }}
impl Drop for Token {{ fn drop(&mut self) {{ println!("drop {{}}", self.n); }} }}
fn make(n: i32) -> impl Tag {{ println!("make {{}}", n); Token {{ n }} }}
{body}
"#
    );
    parse_oracle(
        name,
        &[closure_oracle::Case {
            name,
            source: Box::leak(source.into_boxed_str()),
            expect: closure_oracle::Expect::Run(expected),
        }],
    );
}
#[test]
fn opaque_physical_cleanup_overwrite_and_call_order() {
    opaque_cleanup_order_case(
        "opaque-cleanup-replace",
        r#"
fn main() { let mut x = make(1); x = make(2); println!("value {}", x.id()); }
"#,
        "make 1\nmake 2\ndrop 1\nvalue 2\ndrop 2\n",
    );
}
#[test]
fn opaque_physical_cleanup_preserves_existing_drop_placement() {
    opaque_cleanup_order_case(
        "opaque-cleanup-existing-placement",
        r#"
fn main() { let _retained = Token { n: 9 }; let mut hidden = make(1); hidden = make(2); println!("value {}", hidden.id()); }
"#,
        "make 1\nmake 2\ndrop 1\nvalue 2\ndrop 2\ndrop 9\n",
    );
}
#[test]
fn opaque_physical_cleanup_early_return() {
    opaque_cleanup_order_case(
        "opaque-cleanup-return",
        r#"
fn early(stop: bool) { let x = make(3); if stop { return; } println!("value {}", x.id()); }
fn main() { early(true); early(false); }
"#,
        "make 3\ndrop 3\nmake 3\nvalue 3\ndrop 3\n",
    );
}
#[test]
fn opaque_physical_cleanup_loop_replacement() {
    opaque_cleanup_order_case(
        "opaque-cleanup-loop",
        r#"
fn main() { let mut x = make(0); let mut n = 1i32; while n < 3 { x = make(n); n += 1; } println!("value {}", x.id()); }
"#,
        "make 0\nmake 1\ndrop 0\nmake 2\ndrop 1\nvalue 2\ndrop 2\n",
    );
}
#[test]
fn opaque_physical_cleanup_whole_move_once() {
    opaque_cleanup_order_case(
        "opaque-cleanup-move",
        r#"
fn main() { let x = make(4); let y = x; println!("value {}", y.id()); }
"#,
        "make 4\nvalue 4\ndrop 4\n",
    );
}
#[test]
fn opaque_physical_cleanup_partial_move_remainder_order() {
    opaque_cleanup_order_case(
        "opaque-cleanup-partial",
        r#"
fn main() { let pair = (make(5), make(6)); let (first, _) = pair; println!("value {}", first.id()); }
"#,
        "make 5\nmake 6\nvalue 5\ndrop 5\ndrop 6\n",
    );
}
