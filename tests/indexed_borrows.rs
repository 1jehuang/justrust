//! Borrows of runtime-indexed places (&s[i], &mut s[i], &v.data[i]),
//! references into self via elided method lifetimes, stock-checked.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/oracle.rs"]
mod oracle;
use oracle::{Case, Expect};

#[test]
fn indexed_borrows_match_stock() {
    oracle::check(
        "indexed-borrows",
        &[
            Case {
                name: "shared_index_borrow",
                source: r#"fn main() { let s = [5, 6, 7]; let i = 2; let r = &s[i]; println!("{}", *r); }"#,
                expect: Expect::Run("7\n"),
            },
            Case {
                name: "mut_index_borrow",
                source: r#"fn main() { let mut s = [5, 6, 7]; let i = 1; let r = &mut s[i]; *r += 10; println!("{} {}", s[0], s[1]); }"#,
                expect: Expect::Run("5 16\n"),
            },
            Case {
                name: "field_index_get",
                source: r#"struct V { data: [i32; 4], len: usize }
impl V {
    fn get(&self, i: usize) -> Option<&i32> { if i < self.len { Some(&self.data[i]) } else { None } }
    fn get_mut(&mut self, i: usize) -> &mut i32 { &mut self.data[i] }
}
fn main() {
    let mut v = V { data: [1, 2, 3, 4], len: 3 };
    *v.get_mut(1) += 10;
    match v.get(1) { Some(x) => println!("{}", x), None => println!("none") }
    match v.get(3) { Some(x) => println!("{}", x), None => println!("none") }
}"#,
                expect: Expect::Run("12\nnone\n"),
            },
            Case {
                name: "slice_param_index_borrow",
                source: r#"fn at(s: &[i32], i: usize) -> &i32 { &s[i] }
fn main() { let a = [4, 5, 6]; let s: &[i32] = &a; let r = at(s, 1); println!("{}", *r); }"#,
                expect: Expect::Run("5\n"),
            },
            Case {
                name: "nested_rows_borrow",
                source: r#"struct M { rows: [[i32; 2]; 2] }
impl M { fn at(&self, i: usize, j: usize) -> &i32 { &self.rows[i][j] } }
fn main() { let m = M { rows: [[1, 2], [3, 4]] }; println!("{} {}", m.at(1, 0), m.at(0, 1)); }"#,
                expect: Expect::Run("3 2\n"),
            },
            Case {
                name: "reborrow_through_ref_field",
                source: r#"struct V { data: [i32; 3] }
fn bump(v: &mut V, i: usize) { let r = &mut v.data[i]; *r += 100; }
fn main() { let mut v = V { data: [1, 2, 3] }; bump(&mut v, 2); println!("{}", v.data[2]); }"#,
                expect: Expect::Run("103\n"),
            },
            Case {
                name: "loop_of_element_borrows",
                source: r#"fn main() {
    let mut a = [1, 2, 3, 4];
    let mut i = 0;
    while i < 4 { let r = &mut a[i]; *r *= 2; i += 1; }
    let mut sum = 0;
    let mut j = 0;
    while j < 4 { let r = &a[j]; sum += *r; j += 1; }
    println!("{}", sum);
}"#,
                expect: Expect::Run("20\n"),
            },
            Case {
                name: "two_mut_element_borrows_rejected",
                source: r#"fn main() { let mut a = [1, 2]; let i = 0; let j = 1; let x = &mut a[i]; let y = &mut a[j]; *x += 1; *y += 1; }"#,
                expect: Expect::Invalid("E0499"),
            },
            Case {
                name: "write_while_element_borrowed_rejected",
                source: r#"fn main() { let mut a = [1, 2]; let i = 0; let x = &a[i]; a[1] = 5; println!("{}", x); }"#,
                expect: Expect::Invalid("E0506"),
            },
            Case {
                name: "mut_borrow_of_immutable_rejected",
                source: r#"fn main() { let a = [1, 2]; let i = 0; let x = &mut a[i]; *x = 3; }"#,
                expect: Expect::Invalid("E0596"),
            },
            Case {
                name: "mut_borrow_through_shared_ref_rejected",
                source: r#"fn f(a: &[i32; 2], i: usize) { let x = &mut a[i]; *x = 3; }
fn main() { let a = [1, 2]; f(&a, 0); }"#,
                expect: Expect::Invalid("E0596"),
            },
            Case {
                name: "dangling_local_element_rejected",
                source: r#"fn f(v: &[i32; 2], i: usize) -> &i32 { let a = *v; &a[i] }
fn main() { let v = [1, 2]; let r = f(&v, 0); println!("{}", r); }"#,
                expect: Expect::Invalid("E0515"),
            },
            Case {
                name: "struct_elements_get_and_get_mut",
                source: r#"struct P { x: i32, y: i32 }
struct V { data: [P; 3] }
impl V {
    fn get(&self, i: usize) -> Option<&P> { if i < 3 { Some(&self.data[i]) } else { None } }
    fn get_mut(&mut self, i: usize) -> &mut P { &mut self.data[i] }
}
fn main() {
    let mut v = V { data: [P { x: 1, y: 2 }, P { x: 3, y: 4 }, P { x: 5, y: 6 }] };
    v.get_mut(1).x += 10;
    let i: usize = 2;
    let r = &v.data[i].y;
    println!("{}", r);
    match v.get(1) { Some(p) => println!("{} {}", p.x, p.y), None => println!("none") }
    match v.get(5) { Some(p) => println!("{}", p.x), None => println!("none") }
}"#,
                expect: Expect::Run("6\n13 4\nnone\n"),
            },
            Case {
                name: "move_out_of_borrowed_element_rejected",
                source: r#"struct S { n: String }
fn main() { let v = [S { n: String::new() }]; let i = 0; let r = &v[i]; let s = r.n; println!("{}", s); }"#,
                expect: Expect::Invalid("E0507"),
            },
            Case {
                name: "aggregate_element_alias_rejected",
                source: r#"struct P { x: i32 }
fn main() { let mut v = [P { x: 1 }, P { x: 2 }]; let i = 0; let a = &mut v[i]; let b = &v[1]; a.x += b.x; }"#,
                expect: Expect::Invalid("E0502"),
            },
        ],
    );
}
