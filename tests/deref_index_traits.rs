//! Overloaded `*x` (Deref) and `a[i]` (Index), and `&Self::Target`-style
//! projected reference returns, against stock rustc output.
use justrust_engine::compiler;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

#[path = "support/closure_oracle.rs"]
mod generic_index_oracle;

#[test]
fn generic_index_reference_origins_match_stock() {
    use generic_index_oracle::{Case, Expect};
    generic_index_oracle::check(
        "generic-index-origins",
        &[
            Case {
                name: "vec_reference_origins",
                source: r#"use std::fmt::Display;
fn first<K>(v: &Vec<K>) -> &K { &v[0] }
fn first_mut<K>(v: &mut Vec<K>) -> &mut K { &mut v[0] }
fn clone_first<K: Clone>(v: &Vec<K>) -> K { v[0].clone() }
fn show<K: Display>(v: &Vec<K>) { println!("{}", &v[0]); }
fn main() {
    let mut v = vec![String::from("a"), String::from("b")];
    println!("{} {}", first(&v), clone_first(&v));
    first_mut(&mut v).push('!');
    show(&v);
    println!("{:?}", v);
}"#,
                expect: Expect::Run("a a\na!\n[\"a!\", \"b\"]\n"),
            },
            Case {
                name: "slice_reference_and_method_receivers",
                source: r#"fn slice_first<K>(v: &[K]) -> &K { &v[0] }
fn slice_first_mut<K>(v: &mut [K]) -> &mut K { &mut v[0] }
fn slice_set<K: Copy>(v: &mut [K], i: usize, x: K) { v[i] = x; }
fn slice_clone<K: Clone>(v: &[K]) -> K { v[0].clone() }
fn slice_push(v: &mut [String], i: usize) { v[i].push('!'); }
fn main() {
    let mut v = vec![String::from("a"), String::from("b")];
    println!("{} {}", slice_first(&v), slice_clone(&v));
    slice_first_mut(&mut v).push('?');
    slice_push(&mut v, 1);
    println!("{:?}", v);
    let mut n = vec![2u32, 4];
    slice_set(&mut n, 1, 9);
    println!("{:?}", n);
}"#,
                expect: Expect::Run("a a\n[\"a?\", \"b!\"]\n[2, 9]\n"),
            },
            Case {
                name: "unused_move_out_of_generic_vec",
                source: r#"fn bad<K>(v: &Vec<K>) -> K { v[0] } fn main() {}"#,
                expect: Expect::Invalid("E0507"),
            },
            Case {
                name: "unused_mut_index_from_shared_vec",
                source: r#"fn bad<K>(v: &Vec<K>) -> &mut K { &mut v[0] } fn main() {}"#,
                expect: Expect::Invalid("E0596"),
            },
            Case {
                name: "unused_mut_method_on_shared_slice",
                source: r#"fn bad(v: &[String]) { v[0].push('!'); } fn main() {}"#,
                expect: Expect::Invalid("E0596"),
            },
            Case {
                name: "unused_reference_outlives_vec",
                source: r#"fn bad<K>(v: Vec<K>) -> &'static K { &v[0] } fn main() {}"#,
                expect: Expect::Invalid("E0515"),
            },
        ],
    );
}

#[test]
fn generic_indexed_assignment_match_stock() {
    use generic_index_oracle::{Case, Expect};
    generic_index_oracle::check(
        "generic-indexed-assignment",
        &[
            Case {
                name: "noncopy_vec_element_replacement",
                source: r#"fn set<K>(v: &mut Vec<K>, i: usize, x: K) { v[i] = x; }
fn main() {
    let mut v = vec![String::from("a"), String::from("b")];
    set(&mut v, 1, String::from("c"));
    println!("{:?}", v);
}"#,
                expect: Expect::Run("[\"a\", \"c\"]\n"),
            },
            Case {
                name: "repeated_replacement_drops_selected_element_once",
                source: r#"struct D(u32);
impl Drop for D { fn drop(&mut self) { println!("drop {}", self.0); } }
fn set<K>(v: &mut Vec<K>, i: usize, x: K) { v[i] = x; }
fn main() {
    let mut v = vec![D(1), D(2)];
    set(&mut v, 1, D(3));
    set(&mut v, 1, D(4));
    println!("live {} {}", v[0].0, v[1].0);
    drop(v);
}"#,
                expect: Expect::Run("drop 2\ndrop 3\nlive 1 4\ndrop 1\ndrop 4\n"),
            },
            Case {
                name: "mutable_reference_elements",
                source: r#"fn set<K>(v: &mut Vec<K>, i: usize, x: K) { v[i] = x; }
fn main() {
    let mut a = 1i32;
    let mut b = 2i32;
    let mut c = 3i32;
    let mut d = 4i32;
    let mut v = vec![&mut a];
    let rb = &mut b;
    v.push(rb);
    let rc = &mut c;
    set(&mut v, 0, rc);
    let rd = &mut d;
    set(&mut v, 1, rd);
    *v[0] += 10;
    *v[1] += 20;
    drop(v);
    println!("{} {} {} {}", a, b, c, d);
}"#,
                expect: Expect::Run("1 2 13 24\n"),
            },
            Case {
                name: "aggregate_mutable_reference_elements",
                source: r#"fn set<K>(v: &mut Vec<K>, i: usize, x: K) { v[i] = x; }
fn main() {
    let mut a = 1i32;
    let mut b = 2i32;
    let mut c = 3i32;
    let mut d = 4i32;
    let ra = &mut a;
    let rb = &mut b;
    let old = (ra, rb);
    let mut v = vec![old];
    let rc = &mut c;
    let rd = &mut d;
    let replacement = (rc, rd);
    set(&mut v, 0, replacement);
    *v[0].0 += 10;
    *v[0].1 += 20;
    drop(v);
    println!("{} {} {} {}", a, b, c, d);
}"#,
                expect: Expect::Run("1 2 13 24\n"),
            },
            Case {
                name: "unused_replacement_reuses_moved_element",
                source: r#"fn set<K>(v: &mut Vec<K>, i: usize, x: K) { v[i] = x; }
fn bad(v: &mut Vec<String>, x: String) { set(v, 0, x); println!("{}", x); }
fn main() {}"#,
                expect: Expect::Invalid("E0382"),
            },
            Case {
                name: "unused_replacement_aliases_live_element",
                source: r#"fn set<K>(v: &mut Vec<K>, i: usize, x: K) { v[i] = x; }
fn bad(v: &mut Vec<String>, x: String) {
    let old = &mut v[0]; set(v, 0, x); drop(old);
}
fn main() {}"#,
                expect: Expect::Invalid("E0499"),
            },
            Case {
                name: "unused_replacement_does_not_unify_distinct_families",
                source: r#"fn bad<A, B>(v: &mut Vec<A>, x: B) { v[0] = x; }
fn main() {}"#,
                expect: Expect::Invalid("E0308"),
            },
            Case {
                name: "unused_custom_index_mut_does_not_split_aliases",
                source: r#"use std::ops::{Index, IndexMut};
struct Slots<K> { value: K }
impl<K> Index<usize> for Slots<K> {
    type Output = K;
    fn index(&self, _i: usize) -> &K { &self.value }
}
impl<K> IndexMut<usize> for Slots<K> {
    fn index_mut(&mut self, _i: usize) -> &mut K { &mut self.value }
}
fn bad(v: &mut Slots<String>, x: String) {
    let old = v.index_mut(0); *v.index_mut(1) = x; drop(old);
}
fn main() {}"#,
                expect: Expect::Invalid("E0499"),
            },
            Case {
                name: "unused_assumed_index_mut_does_not_issue_native_role",
                source: r#"use std::ops::IndexMut;
fn bad<C: IndexMut<usize, Output = String>>(v: &mut C, x: String) {
    let old = v.index_mut(0); *v.index_mut(1) = x; drop(old);
}
fn main() {}"#,
                expect: Expect::Invalid("E0499"),
            },
            Case {
                name: "unused_replacement_escapes_short_reference",
                source: r#"fn set<K>(v: &mut Vec<K>, i: usize, x: K) { v[i] = x; }
fn bad<'a>(v: &mut Vec<&'a str>) { let s = String::from("short"); set(v, 0, &s); }
fn main() {}"#,
                expect: Expect::Invalid("E0597"),
            },
        ],
    );
}

fn run(source: &str, expected: &str) {
    for workers in [1, 8] {
        let compiled = compiler::compile(source.into(), workers, false)
            .unwrap_or_else(|error| panic!("{error}\n{source}"));
        let path = std::env::temp_dir().join(format!(
            "jr-deref-index-{}-{workers}-{}",
            std::process::id(),
            source.len()
        ));
        fs::write(&path, compiled.executable.unwrap()).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        let output = Command::new(&path).output().unwrap();
        let _ = fs::remove_file(&path);
        assert!(output.status.success());
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            expected,
            "{source}"
        );
    }
}

#[test]
fn projected_reference_return() {
    run(
        "trait G { type T; fn get(&self) -> &Self::T; }
struct W { v: i32 }
impl G for W { type T = i32; fn get(&self) -> &i32 { &self.v } }
fn main() { let w = W { v: 5 }; let x = *w.get() + 1; println!(\"{}\", x); }",
        "6\n",
    );
}

#[test]
fn boxed_generic_trait_receiver_preserves_relaxed_target() {
    let source = r#"
trait Shape { fn area(&self) -> f64; }
struct Square(f64);
impl Shape for Square { fn area(&self) -> f64 { self.0 * self.0 } }
fn one<S: Shape + ?Sized>(s: &Box<S>) -> f64 { s.area() }
fn sized<S: Shape>(s: &Box<S>) -> f64 { s.area() }
fn target<S: ?Sized>(s: &Box<S>) -> &S { &**s }
fn through_ref<S: Shape + ?Sized>(s: &S) -> f64 { s.area() }
fn main() {
    let a = Box::new(Square(2.0));
    let b: Box<dyn Shape> = Box::new(Square(3.0));
    println!("{} {} {} {}", one(&a), sized(&a), one(&b), through_ref(target(&b)));
}
"#;
    for workers in [1, 8] {
        let checked = compiler::compile(source.into(), workers, true).unwrap();
        assert!(checked.executable.is_none());
    }
    run(source, "4 4 9 9\n");
}

#[test]
fn relaxed_deref_target_does_not_grant_missing_traits_or_sizing() {
    for source in [
        // Even unused bodies must prove Shape for their exact receiver.
        "trait Shape { fn area(&self) -> f64; } fn unused<S: ?Sized>(s: &Box<S>) -> f64 { s.area() } fn main() {}",
        // A dynamic receiver never grants a missing concrete vtable.
        "trait Shape { fn area(&self) -> f64; } fn unused() { let _: Box<dyn Shape> = Box::new(0i32); } fn main() {}",
        // A relaxed target bound does not make an unrelated associated type
        // implicitly ?Sized, nor prove an explicit Sized obligation.
        "trait Has { type Out; } struct Wrap<T: ?Sized>{ p: *const T } impl<T: ?Sized> Has for Wrap<T> { type Out = T; } fn main() {}",
        "trait Has { type Out: ?Sized + Copy; } struct Wrap<T: ?Sized>{ p: *const T } impl<T: ?Sized> Has for Wrap<T> { type Out = T; } fn main() {}",
        "fn needs_sized<T>(s: &T) {} fn unused<S: ?Sized>(s: &Box<S>) { needs_sized(&**s); } fn main() {}",
    ] {
        for workers in [1, 8] {
            assert!(
                compiler::compile(source.into(), workers, true).is_err(),
                "workers={workers}: {source}"
            );
        }
    }
}

#[test]
fn overloaded_deref_and_index() {
    run(
        "use std::ops::{Deref, Index};
struct W { v: i32 }
impl Deref for W { type Target = i32; fn deref(&self) -> &i32 { &self.v } }
struct A { a: [i32; 3] }
impl Index<usize> for A { type Output = i32; fn index(&self, i: usize) -> &i32 { if i == 1 { &self.a[1] } else { &self.a[2] } } }
fn main() { let w = W { v: 5 }; let x = *w + 1; let a = A { a: [1, 2, 3] }; let i: usize = 2; let s = a[1] + a[i]; let one = a[1]; println!(\"{} {} {}\", x, s, one); }",
        "6 5 2\n",
    );
}

#[test]
fn missing_deref_is_rejected() {
    let source =
        "struct W { v: i32 } fn main() { let w = W { v: 1 }; let x = *w; println!(\"{}\", x); }";
    assert!(compiler::compile(source.into(), 1, true).is_err());
}

#[test]
fn box_deref_mut_assignment() {
    run(
        "struct P { x: i32 } fn main() { let mut b = Box::new(5); *b += 1; *b = *b * 2; let mut p = Box::new(P { x: 1 }); (*p).x += 4; println!(\"{} {}\", *b, p.x); }",
        "12 5\n",
    );
    let immutable = "fn main() { let b = Box::new(5); *b += 1; println!(\"{}\", *b); }";
    assert!(compiler::compile(immutable.into(), 1, true).is_err());
}

#[test]
fn box_deref() {
    run(
        "fn main() { let b = Box::new(5); let x = *b + 1; println!(\"{}\", x); }",
        "6\n",
    );
}

#[test]
fn deref_coercion_at_call_and_let_sites() {
    run(
        "struct N { v: i32 }
fn get(n: &N) -> i32 { n.v }
fn main() { let b = Box::new(N { v: 7 }); let bb = Box::new(Box::new(N { v: 2 })); let r: &N = &b; println!(\"{} {} {}\", get(&b), get(&bb), r.v); }",
        "7 2 7\n",
    );
}

#[test]
fn bounds_on_structs_and_enums() {
    run(
        "use std::fmt::Display;
struct Sorted<T: Ord> { v: T }
enum E<T> where T: Copy { A(T), B }
impl<T: Ord + Display> Sorted<T> { fn show(&self) -> &T { &self.v } }
fn main() { let s = Sorted { v: 3 }; let e: E<i32> = E::A(4); let n = match e { E::A(x) => x, E::B => 0 }; println!(\"{} {}\", s.show(), n); }",
        "3 4\n",
    );
    let bad = "struct S<T: Ord> { v: T }\nstruct F(f64);\nfn main() { let s = S { v: F(1.0) }; let _ = s; }";
    let error = compiler::compile(bad.into(), 1, true)
        .err()
        .expect("accepted");
    assert!(error.to_string().contains("E0277"), "{error}");
    let copy_bad = "struct S<T: Copy> { v: T } struct F { v: i32 } fn main() { let s = S { v: F { v: 1 } }; let _ = s; }";
    let error = compiler::compile(copy_bad.into(), 1, true)
        .err()
        .expect("accepted missing Copy bound");
    assert!(error.to_string().contains("E0277"), "{error}");
    let unused =
        "struct S<T: Copy> { v: T } struct F { v: i32 } fn unused(x: &S<F>) {} fn main() {}";
    let error = compiler::compile(unused.into(), 1, true)
        .err()
        .expect("accepted unused ill-formed signature");
    assert!(error.to_string().contains("E0277"), "{error}");
}

#[test]
fn explicit_deref_of_nominal_target() {
    // `*p`, `(*p).field`, `&*p` with a struct Target (stock: "4 4 4").
    run(
        "#[derive(Clone, Copy)]
struct W { v: i32 }
struct P { w: W }
impl std::ops::Deref for P { type Target = W; fn deref(&self) -> &W { &self.w } }
fn main() { let p = P { w: W { v: 4 } }; let c = *p; let f = (*p).v; let r = &*p; println!(\"{} {} {}\", c.v, f, r.v); }",
        "4 4 4\n",
    );
}

#[test]
fn deref_borrow_without_impl_is_rejected() {
    let source =
        "struct P { v: i32 } fn main() { let p = P { v: 1 }; let r = &*p; println!(\"{}\", r.v); }";
    assert!(compiler::compile(source.into(), 1, true).is_err());
}

#[test]
fn method_and_field_autoderef_through_deref() {
    // Stock rustc 1.98.1: "4 4 9 9".
    run(
        "struct W { v: i32 }
impl W { fn get(&self) -> i32 { self.v } }
struct P { w: W }
impl std::ops::Deref for P { type Target = W; fn deref(&self) -> &W { &self.w } }
fn main() { let p = P { w: W { v: 4 } }; let a = p.get(); let b = p.v; let x = Box::new(W { v: 9 }); let c = x.get(); let d = x.v; println!(\"{} {} {} {}\", a, b, c, d); }",
        "4 4 9 9\n",
    );
}

#[test]
fn autoderef_without_member_is_rejected() {
    let source = "struct W { v: i32 }
struct P { w: W }
impl std::ops::Deref for P { type Target = W; fn deref(&self) -> &W { &self.w } }
fn main() { let p = P { w: W { v: 4 } }; let b = p.missing; println!(\"{}\", b); }";
    assert!(compiler::compile(source.into(), 1, true).is_err());
}

#[test]
fn index_mut_through_deref_mut_guards() {
    run(
        "use std::cell::RefCell;
fn main(){let c=RefCell::new(vec![1u32,2]);
{let mut g=c.borrow_mut();let r=&mut g[0];*r+=5;}
{let mut g=c.borrow_mut();g[1]=9;}
{let mut b=Box::new(vec![3u32]);b[0]+=1;let r=&mut b[0];*r*=2;println!(\"{}\",b[0]);}
println!(\"{:?}\",c.borrow());}",
        "8\n[6, 9]\n",
    );
}

#[test]
fn boxed_array_index_reads_through_deref() {
    run(
        "fn main(){let b=Box::new([1u32,2,3]);let i=2;let x=b[i]+b[0];let y=b[1];println!(\"{} {}\",x,y);println!(\"{} {}\",b[i],b[1]);}",
        "4 2\n3 2\n",
    );
}

#[test]
fn boxed_array_index_assignment_through_deref_mut() {
    run(
        "fn main(){let mut b=Box::new([1u32,2,3]);let i=2;b[0]=5;b[i]+=10;b[1]*=4;println!(\"{} {} {}\",b[0],b[1],b[2]);}",
        "5 8 13\n",
    );
}

#[test]
fn boxed_array_index_borrows_through_deref() {
    run(
        "fn main(){let mut b=Box::new([1u32,2,3]);let i=2;{let r=&mut b[i];*r+=5;}let m=&mut b[0];*m=9;let x=&b[i];let y=&b[1];println!(\"{} {} {}\",x,y,b[0]);}",
        "8 2 9\n",
    );
    for source in [
        "fn main(){let mut b=Box::new([1u32,2,3]);let x=&b[1];let m=&mut b[0];*m=1;println!(\"{}\",x);}",
        "fn main(){let b=Box::new([1u32,2,3]);let m=&mut b[0];*m=1;}",
        "fn main(){let b=Box::new([1u32,2,3]);let x=&b[3];println!(\"{}\",x);}",
    ] {
        assert!(
            compiler::compile(source.into(), 1, true).is_err(),
            "{source}"
        );
    }
}

#[test]
fn inferred_string_local_deref_coerces_at_str_argument() {
    run(
        "fn main(){let mut s=String::new();for i in 0..3{s.push_str(&i.to_string());}let n=7u32;let t=n.to_string();s.push_str(&t);println!(\"{}\",s);}",
        "0127\n",
    );
    let source = "fn main(){let mut s=String::new();let t=5u32;s.push_str(&t);println!(\"{}\",s);}";
    assert!(compiler::compile(source.into(), 1, true).is_err());
}

#[test]
fn user_index_with_tuple_struct_and_str_operands() {
    run(
        "use std::ops::{Index, IndexMut};
struct Grid { w: usize, cells: Vec<char> }
impl Index<(usize, usize)> for Grid { type Output = char; fn index(&self, (r, c): (usize, usize)) -> &char { &self.cells[r * self.w + c] } }
impl IndexMut<(usize, usize)> for Grid { fn index_mut(&mut self, (r, c): (usize, usize)) -> &mut char { &mut self.cells[r * self.w + c] } }
#[derive(Clone, Copy)] struct Pos { r: usize, c: usize }
struct Board { w: usize, v: Vec<i64> }
impl Index<Pos> for Board { type Output = i64; fn index(&self, p: Pos) -> &i64 { &self.v[p.r * self.w + p.c] } }
impl IndexMut<Pos> for Board { fn index_mut(&mut self, p: Pos) -> &mut i64 { &mut self.v[p.r * self.w + p.c] } }
struct Env { names: Vec<String>, vals: Vec<i32> }
impl Index<&str> for Env { type Output = i32; fn index(&self, k: &str) -> &i32 { let mut i = 0; while self.names[i] != k { i += 1; } &self.vals[i] } }
impl IndexMut<&str> for Env { fn index_mut(&mut self, k: &str) -> &mut i32 { let mut i = 0; while self.names[i] != k { i += 1; } &mut self.vals[i] } }
struct Cube { v: Vec<u8> }
impl Index<(usize, (usize, usize))> for Cube { type Output = u8; fn index(&self, (a, (b, c)): (usize, (usize, usize))) -> &u8 { &self.v[a * 4 + b * 2 + c] } }
fn main() {
    let mut g = Grid { w: 3, cells: vec!['.'; 9] };
    g[(1, 1)] = '#'; g[(2, 0)] = '@';
    for r in 0..3 { println!(\"{}\", (0..3).map(|c| g[(r, c)]).collect::<String>()); }
    let mut b = Board { w: 2, v: vec![1, 2, 3, 4] };
    b[Pos { r: 1, c: 0 }] += 10; b[Pos { r: 0, c: 1 }] *= 7; b[Pos { r: 1, c: 1 }] = -5;
    let p = Pos { r: 1, c: 0 };
    println!(\"{} {} {}\", b[p], b[Pos { r: 0, c: 1 }], b[Pos { r: 1, c: 1 }]);
    let mut e = Env { names: vec![\"x\".to_string(), \"y\".to_string()], vals: vec![1, 2] };
    e[\"y\"] += 40; e[\"x\"] = 9;
    let k = String::from(\"y\");
    println!(\"{} {}\", e[\"x\"], e[k.as_str()]);
    let c = Cube { v: vec![0, 1, 2, 3, 4, 5, 6, 7] };
    println!(\"{} {}\", c[(1, (0, 1))], c[(0, (1, 1))]);
}",
        "...\n.#.\n@..\n13 14 -5\n9 42\n5 3\n",
    );
}

#[test]
fn user_index_with_range_operand_and_unsized_output() {
    run(
        "use std::ops::{Index, Range};
struct B { v: Vec<i64> }
impl Index<Range<usize>> for B { type Output = [i64]; fn index(&self, r: Range<usize>) -> &[i64] { &self.v[r.start..r.end] } }
impl Index<usize> for B { type Output = i64; fn index(&self, i: usize) -> &i64 { &self.v[i] } }
struct S { v: Vec<i64> }
impl Index<Range<usize>> for S { type Output = i64; fn index(&self, r: Range<usize>) -> &i64 { &self.v[r.start + r.end] } }
fn main() { let b = B { v: vec![1, 2, 3, 4] }; let r = 1..3usize; let s = &b[r]; println!(\"{:?} {}\", s, b[3]);
    let t = &b[(0..2)]; let s = S { v: vec![5, 6, 7, 8] }; let n = 1; println!(\"{:?} {} {}\", t, s[(1..2)], s[(n..n + 1)]); }",
        "[2, 3] 4\n[1, 2] 8 8\n",
    );
}

#[test]
fn user_index_operand_mismatch_is_e0277() {
    let head = "use std::ops::Index; struct G { w: usize, v: Vec<u8> }
impl Index<(usize, usize)> for G { type Output = u8; fn index(&self, p: (usize, usize)) -> &u8 { &self.v[p.0 * self.w + p.1] } }
fn g() -> G { G { w: 2, v: vec![1, 2, 3, 4] } }";
    for body in [
        "fn main() { let a = g(); let _ = a[(1u8, 2u8)]; }",
        "fn main() { let a = g(); let _ = a[\"a\"]; }",
        "fn main() { let a = g(); let _ = a[(1, 2, 3)]; }",
        "fn main() { let a = g(); let _ = a[3]; }",
        "fn main() { let mut a = g(); a[(0, 0)] = 2; }",
        "fn main() { let mut a = g(); a[(0, 0)] += 2; }",
        "struct N; fn main() { let n = N; let _ = n[(1, 2)]; }",
    ] {
        let source = format!("{head}\n{body}");
        let error = match compiler::compile(source.clone(), 1, true) {
            Err(error) => error.to_string(),
            Ok(_) => panic!("accepted {source}"),
        };
        assert!(error.contains("E0277"), "{error}\n{source}");
    }
}
