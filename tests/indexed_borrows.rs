//! Borrows of runtime-indexed places (&s[i], &mut s[i], &v.data[i]),
//! references into self via elided method lifetimes, stock-checked.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/oracle.rs"]
mod oracle;
use oracle::{Case, Expect};

#[allow(dead_code)]
#[path = "support/closure_oracle.rs"]
mod closure_oracle;
#[path = "support/stock_rustc.rs"]
mod stock_rustc;

#[test]
fn scalar_cell_shared_storage_effects_match_stock() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check_with_compiler(
        "scalar-cell-shared-storage",
        &[
            Case {
                name: "shared_aliases_observe_original_storage",
                source: r#"use std::cell::Cell;
fn main(){let cell=Cell::new(3usize);let a=&cell;let b=&cell;a.set(7usize);println!("{} {}",b.get(),cell.get());b.set(11usize);println!("{}",a.get());}"#,
                expect: Expect::Run("7 7\n11\n"),
            },
            Case {
                name: "reference_parameter_mutates_original_cell",
                source: r#"use core::cell::Cell;fn set(cell:&Cell<usize>,value:usize){cell.set(value);}fn get(cell:&Cell<usize>)->usize{cell.get()}
fn main(){let cell=Cell::new(2usize);let a=&cell;set(a,13usize);println!("{} {}",get(a),cell.get());}"#,
                expect: Expect::Run("13 13\n"),
            },
            Case {
                name: "owned_move_keeps_distinct_storage",
                source: r#"use core::cell::Cell;fn main(){let first=Cell::new(4usize);let second=first;let other=Cell::new(9usize);second.set(6usize);println!("{} {}",second.get(),other.get());}"#,
                expect: Expect::Run("6 9\n"),
            },
            Case {
                name: "small_and_wide_scalar_memory_widths",
                source: r#"use core::cell::Cell;fn main(){let byte=Cell::new(255u8);let flag=Cell::new(false);let letter=Cell::new('a');let wide=Cell::new(18446744073709551617u128);let real=Cell::new(1.5f64);byte.set(7u8);flag.set(true);letter.set('z');wide.set(36893488147419103235u128);real.set(2.5f64);assert_eq!(byte.get(),7u8);assert!(flag.get());assert_eq!(letter.get(),'z');assert_eq!(wide.get(),36893488147419103235u128);assert_eq!(real.get(),2.5f64);println!("scalar cell widths");}"#,
                expect: Expect::Run("scalar cell widths\n"),
            },
            Case {
                name: "shared_reborrow_keeps_exclusive_parent",
                source: r#"use core::cell::Cell;fn main(){let mut cell=Cell::new(1usize);let exclusive=&mut cell;let shared=&*exclusive;shared.set(8usize);println!("{}",shared.get());exclusive.set(9usize);println!("{}",cell.get());}"#,
                expect: Expect::Run("8\n9\n"),
            },
            Case {
                name: "shared_closure_and_associated_operations",
                source: r#"use core::cell::Cell;fn next(cell:&Cell<usize>)->usize{println!("value");cell.get()+1usize}fn main(){let cell=Cell::new(4usize);let observe=||cell.get();let write=||Cell::set(&cell,next(&cell));write();println!("{} {}",observe(),Cell::get(&cell));}"#,
                expect: Expect::Run("value\n5 5\n"),
            },
            Case {
                name: "cell_is_not_copy",
                source: r#"use core::cell::Cell;fn main(){let cell=Cell::new(1usize);let moved=cell;println!("{} {}",cell.get(),moved.get());}"#,
                expect: Expect::Invalid("E0382"),
            },
            Case {
                name: "exclusive_alias_still_conflicts",
                source: r#"use core::cell::Cell;fn main(){let mut cell=Cell::new(1usize);let shared=&cell;let exclusive=&mut cell;shared.set(2usize);println!("{}",exclusive.get());}"#,
                expect: Expect::Invalid("E0502"),
            },
            Case {
                name: "live_shared_cell_prevents_owner_move",
                source: r#"use core::cell::Cell;fn main(){let cell=Cell::new(1usize);let shared=&cell;let moved=cell;shared.set(2usize);println!("{}",moved.get());}"#,
                expect: Expect::Invalid("E0505"),
            },
            Case {
                name: "cell_alias_cannot_outlive_owner",
                source: r#"use core::cell::Cell;fn main(){let shared;{let cell=Cell::new(1usize);shared=&cell;}shared.set(2usize);}"#,
                expect: Expect::Invalid("E0597"),
            },
        ],
        std::path::Path::new(&stock_rustc::stock_rustc()),
    );
}

#[test]
fn explicit_indexed_field_borrows_match_stock() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check_with_compiler(
        "explicit-indexed-field-borrows",
        &[
            Case {
                name: "mutable_constant_and_shared_dynamic_fields",
                source: r#"struct Record { name: String }
fn index()->usize { println!("index"); 0 }
fn main() { let mut records=vec![Record{name:String::from("hello")}]; let name=&mut records[0].name; name.push_str("!"); let shared=&records[index()].name; println!("{}",shared.as_str()); }"#,
                expect: Expect::Run("index\nhello!\n"),
            },
            Case {
                name: "mutable_dynamic_field_evaluated_once",
                source: r#"struct Record { name: String }
fn index()->usize { println!("index"); 0 }
fn main() { let mut records=vec![Record{name:String::from("hello")}]; let name=&mut records[index()].name; name.push_str("!"); println!("{}",records[0].name.as_str()); }"#,
                expect: Expect::Run("index\nhello!\n"),
            },
            Case {
                name: "shared_owner_rejects_mutable_field_borrow",
                source: r#"struct Record { name: String }
fn main() { let records=vec![Record{name:String::from("hello")}]; let shared=&records; let name=&mut shared[0].name; name.push_str("!"); }"#,
                expect: Expect::Invalid("E0596"),
            },
            Case {
                name: "overlapping_indexed_field_loans_reject",
                source: r#"struct Record { name: String }
fn main() { let mut records=vec![Record{name:String::from("hello")}]; let a=&mut records[0].name; let b=&mut records[0].name; a.push_str(b.as_str()); }"#,
                expect: Expect::Invalid("E0499"),
            },
        ],
        std::path::Path::new(&stock_rustc::stock_rustc()),
    );
}

#[test]
fn indexed_field_method_receivers_match_stock() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check_with_compiler(
        "indexed-field-method-receivers",
        &[
            Case {
                name: "vec_noncopy_field_shared_receiver",
                source: r#"struct Record { name: String }
fn main() { let records=vec![Record{name:"hello".into()}]; println!("{}",records[0].name.as_str()); println!("{}",records[0].name.as_str()); }"#,
                expect: Expect::Run("hello\nhello\n"),
            },
            Case {
                name: "vec_nested_field_mutable_receiver",
                source: r#"struct Inner { name: String } struct Record { inner: Inner }
fn main() { let mut records=vec![Record{inner:Inner{name:String::from("hello")}}]; records[0].inner.name.push_str("!"); println!("{}",records[0].inner.name.as_str()); }"#,
                expect: Expect::Run("hello!\n"),
            },
            Case {
                name: "custom_index_nested_field_evaluates_index_once",
                source: r#"use std::ops::{Index,IndexMut};
struct Value { text: String } struct Record { value: Value } struct Store { record: Record }
impl Index<usize> for Store { type Output=Record; fn index(&self,_i:usize)->&Record { &self.record } }
impl IndexMut<usize> for Store { fn index_mut(&mut self,_i:usize)->&mut Record { &mut self.record } }
fn index()->usize { println!("index"); 0 }
fn main() { let mut store=Store{record:Record{value:Value{text:String::from("hello")}}}; store[index()].value.text.push_str("!"); println!("{}",store[index()].value.text.as_str()); }"#,
                expect: Expect::Run("index\nindex\nhello!\n"),
            },
            Case {
                name: "reference_vec_field_receivers",
                source: r#"struct Record { name: String }
fn read(records:&Vec<Record>) { println!("{}",records[0].name.as_str()); let i=0usize; println!("{}",records[i].name.as_str()); }
fn change(records:&mut Vec<Record>) { records[0].name.push_str("!"); }
fn main() { let mut records=vec![Record{name:String::from("hello")}]; read(&records); change(&mut records); read(&records); }"#,
                expect: Expect::Run("hello\nhello\nhello!\nhello!\n"),
            },
            Case {
                name: "shared_vec_implicit_index_cannot_mutate_field",
                source: r#"struct Record { name: String }
fn main() { let records=vec![Record{name:String::from("hello")}]; let shared=&records; shared[0].name.push_str("!"); }"#,
                expect: Expect::Invalid("E0596"),
            },
            Case {
                name: "shared_vec_cannot_mutate_field",
                source: r#"struct Record { name: String }
fn main() { let records=vec![Record{name:String::from("hello")}]; let shared=&records; (*shared)[0].name.push_str("!"); }"#,
                expect: Expect::Invalid("E0596"),
            },
        ],
        std::path::Path::new(&stock_rustc::stock_rustc()),
    );
}

#[test]
fn returned_reference_reborrows_match_stock() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check_with_compiler(
        "returned-reference-reborrows",
        &[
            Case {
                name: "shared_noncopy_reference_not_moved",
                source: r#"struct Value { n: i32 }
fn get(v: &Value) -> &Value { v }
fn main() { let v=Value { n: 42 }; let r=&*get(&v); assert_eq!(r.n,42); assert_eq!(v.n,42); }"#,
                expect: Expect::Run(""),
            },
            Case {
                name: "mutable_reference_preserves_aliasing",
                source: r#"fn get(v: &mut i32) -> &mut i32 { v }
fn main() { let mut n=1; let r=&mut *get(&mut n); *r=42; assert_eq!(n,42); }"#,
                expect: Expect::Run(""),
            },
            Case {
                name: "call_evaluated_once",
                source: r#"struct Value { n: i32 }
impl Value { fn get(&mut self)->&i32 { self.n+=1; &self.n } }
fn main() { let mut v=Value { n: 1 }; let r=&*v.get(); assert_eq!(*r,2); assert_eq!(v.n,2); }"#,
                expect: Expect::Run(""),
            },
            Case {
                name: "returned_error_kind_assertion",
                source: r#"use std::num::IntErrorKind;
fn main() { assert_eq!(*u8::from_str_radix("100",16).unwrap_err().kind(),IntErrorKind::PosOverflow); }"#,
                expect: Expect::Run(""),
            },
            Case {
                name: "shared_result_cannot_be_mutated",
                source: "fn get(v:&i32)->&i32{v}fn main(){let n=1;let r=&mut *get(&n);*r=2;}",
                expect: Expect::Invalid("E0596"),
            },
            Case {
                name: "overlapping_mutable_reborrow",
                source: "fn get(v:&mut i32)->&mut i32{v}fn main(){let mut n=1;let r=&mut *get(&mut n);let s=&mut n;*r+=*s;}",
                expect: Expect::Invalid("E0499"),
            },
            Case {
                name: "reborrow_cannot_outlive_owner",
                source: "fn get(v:&i32)->&i32{v}fn main(){let r;{let n=1;r=&*get(&n);}println!(\"{}\",r);}",
                expect: Expect::Invalid("E0597"),
            },
        ],
        std::path::Path::new(&stock_rustc::stock_rustc()),
    );
}

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
