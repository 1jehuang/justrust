//! Borrows of runtime-indexed places (&s[i], &mut s[i], &v.data[i]),
//! references into self via elided method lifetimes, stock-checked.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/oracle.rs"]
mod oracle;
use oracle::{Case, Expect};

#[allow(dead_code)]
#[path = "support/closure_oracle.rs"]
mod closure_oracle;
use closure_oracle::pinned_stock as stock_rustc;

#[test]
fn explicit_local_deref_refcell_field_borrows_match_stock() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check_with_compiler(
        "explicit-local-deref-refcell-field-borrows",
        &[
            Case {
                name: "authored_concrete_guard_entries_iteration",
                source: r#"use std::cell::RefCell;struct Account{entries:Vec<String>}
fn main(){let account=RefCell::new(Account{entries:vec![String::from("first"),String::from("second")]});
let view=account.borrow();for entry in &view.entries{println!("{}",entry);}println!("{}",view.entries.len());}"#,
                expect: Expect::Run("first\nsecond\n2\n"),
            },
            Case {
                name: "authored_concrete_mutable_guard_field_borrow",
                source: r#"use std::cell::RefCell;struct Account{entries:Vec<String>}
fn main(){let account=RefCell::new(Account{entries:vec![String::from("first")]});
{let mut view=account.borrow_mut();let entries=&mut view.entries;entries.push(String::from("second"));}
let view=account.borrow();for entry in &view.entries{println!("{}",entry);}}"#,
                expect: Expect::Run("first\nsecond\n"),
            },
            Case {
                name: "stored_field_precedence_does_not_call_deref",
                source: r#"use std::ops::Deref;struct Target{value:i32}struct Owner{value:i32,target:Target}
impl Deref for Owner{type Target=Target;fn deref(&self)->&Target{println!("deref");&self.target}}
fn main(){let owner=Owner{value:3i32,target:Target{value:7i32}};let value=&owner.value;println!("{}",*value);}"#,
                expect: Expect::Run("3\n"),
            },
            Case {
                name: "unused_shared_guard_cannot_mutably_borrow_field",
                source: r#"use std::cell::RefCell;struct Account{entries:Vec<String>}
fn unused(account:&RefCell<Account>){let mut view=account.borrow();let entries=&mut view.entries;entries.push(String::new());}fn main(){}"#,
                expect: Expect::Invalid("E0596"),
            },
            Case {
                name: "unused_missing_guard_target_field_rejects",
                source: r#"use std::cell::RefCell;struct Account{entries:Vec<String>}
fn unused(account:&RefCell<Account>){let view=account.borrow();let entries=&view.missing;}fn main(){}"#,
                expect: Expect::Invalid("E0609"),
            },
            Case {
                name: "unused_private_deref_target_field_rejects",
                source: r#"use std::ops::Deref;mod hidden{pub struct Target{value:i32}}
struct Owner{target:hidden::Target}impl Deref for Owner{type Target=hidden::Target;fn deref(&self)->&hidden::Target{&self.target}}
fn unused(owner:Owner){let value=&owner.value;}fn main(){}"#,
                expect: Expect::Invalid("E0616"),
            },
            Case {
                name: "unused_guard_field_borrow_requires_mutable_owner",
                source: r#"use std::cell::RefCell;struct Account{entries:Vec<String>}
fn unused(account:&RefCell<Account>){let view=account.borrow_mut();let entries=&mut view.entries;entries.push(String::new());}fn main(){}"#,
                expect: Expect::Invalid("E0596"),
            },
            Case {
                name: "unused_overlapping_guard_field_borrows_reject",
                source: r#"use std::cell::RefCell;struct Account{entries:Vec<String>}
fn unused(account:&RefCell<Account>){let mut view=account.borrow_mut();let a=&mut view.entries;let b=&mut view.entries;a.push(String::new());b.push(String::new());}fn main(){}"#,
                expect: Expect::Invalid("E0499"),
            },
        ],
        std::path::Path::new(&stock_rustc::stock_rustc()),
    );
}

#[test]
fn indexed_formatting_preserves_field_authority() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check_with_compiler(
        "indexed-formatting-field-authority",
        &[
            Case {
                name: "stored_field_precedes_deref_target_index",
                source: r#"use std::ops::Deref;struct Target{values:[i32;2]}struct Owner{values:[i32;2],target:Target}
impl Deref for Owner{type Target=Target;fn deref(&self)->&Target{println!("deref");&self.target}}
fn main(){let owner=Owner{values:[3i32,5i32],target:Target{values:[7i32,9i32]}};println!("{}",owner.values[1]);}"#,
                expect: Expect::Run("5\n"),
            },
            Case {
                name: "deref_field_index_evaluates_once",
                source: r#"use std::ops::Deref;struct Target{values:[i32;2]}struct Owner{target:Target}
impl Deref for Owner{type Target=Target;fn deref(&self)->&Target{println!("deref");&self.target}}
fn index()->usize{println!("index");1usize}fn main(){let owner=Owner{target:Target{values:[7i32,9i32]}};println!("{}",owner.values[index()]);}"#,
                expect: Expect::Run("deref\nindex\n9\n"),
            },
            Case {
                name: "stored_tuple_field_precedes_deref_target",
                source: r#"use std::ops::Deref;struct Owner([i32;2],([i32;2],));
impl Deref for Owner{type Target=([i32;2],);fn deref(&self)->&Self::Target{println!("deref");&self.1}}
fn main(){let owner=Owner([3i32,5i32],([7i32,9i32],));println!("{}",owner.0[1]);}"#,
                expect: Expect::Run("5\n"),
            },
            Case {
                name: "private_stored_field_does_not_fall_back",
                source: r#"mod hidden{pub struct Owner{values:[i32;2]}}fn unused(owner:hidden::Owner){println!("{}",owner.values[0]);}fn main(){}"#,
                expect: Expect::Invalid("E0616"),
            },
        ],
        std::path::Path::new(&stock_rustc::stock_rustc()),
    );
}

#[test]
fn explicit_indexed_deref_borrows_reference_free_match_stock() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check_with_compiler(
        "explicit-indexed-deref-reference-free",
        &[
            Case {
                name: "generic_indexed_borrow_value_arguments",
                source: r#"use std::ops::{Deref,DerefMut};struct Target<T>{values:[T;2]}struct Owner<T>{target:Target<T>}
impl<T> Deref for Owner<T>{type Target=Target<T>;fn deref(&self)->&Target<T>{&self.target}}
impl<T> DerefMut for Owner<T>{fn deref_mut(&mut self)->&mut Target<T>{&mut self.target}}
fn read<T:Copy>(value:&T)->T{*value}fn change(value:&mut i32){*value+=10;}
fn main(){let mut owner:Owner<i32>=Owner{target:Target{values:[3i32,7i32]}};let before=read(&owner.values[1]);change(&mut owner.values[0]);println!("{} {}",before,owner.values[0]);}"#,
                expect: Expect::Run("7 13\n"),
            },
            Case {
                name: "tuple_indexed_borrow_value_arguments",
                source: r#"use std::ops::{Deref,DerefMut};struct Owner{target:([i32;2],i32)}
impl Deref for Owner{type Target=([i32;2],i32);fn deref(&self)->&Self::Target{&self.target}}
impl DerefMut for Owner{fn deref_mut(&mut self)->&mut Self::Target{&mut self.target}}
fn read(value:&i32)->i32{*value}fn change(value:&mut i32){*value+=10;}
fn main(){let mut owner=Owner{target:([3i32,7i32],11i32)};let before=read(&owner.0[1]);change(&mut owner.0[0]);println!("{} {}",before,owner.0[0]);}"#,
                expect: Expect::Run("7 13\n"),
            },
            Case {
                name: "unused_immutable_indexed_owner_rejects",
                source: r#"use std::ops::{Deref,DerefMut};struct Target{values:[i32;2]}struct Owner{target:Target}
impl Deref for Owner{type Target=Target;fn deref(&self)->&Target{&self.target}}
impl DerefMut for Owner{fn deref_mut(&mut self)->&mut Target{&mut self.target}}
fn unused(owner:Owner){let value=&mut owner.values[0];*value=9;}fn main(){}"#,
                expect: Expect::Invalid("E0596"),
            },
            Case {
                name: "unused_overlapping_indexed_owner_rejects",
                source: r#"use std::ops::{Deref,DerefMut};struct Target{values:[i32;2]}struct Owner{target:Target}
impl Deref for Owner{type Target=Target;fn deref(&self)->&Target{&self.target}}
impl DerefMut for Owner{fn deref_mut(&mut self)->&mut Target{&mut self.target}}
fn unused(mut owner:Owner){let a=&mut owner.values[0];let b=&mut owner.values[0];*a+=*b;}fn main(){}"#,
                expect: Expect::Invalid("E0499"),
            },
        ],
        std::path::Path::new(&stock_rustc::stock_rustc()),
    );
}

#[test]
fn returned_promotion_unproven_scope_is_explicit_unsupported() {
    use std::{fs, process::Command};
    let directory =
        std::env::temp_dir().join(format!("jr-promotion-boundaries-{}", std::process::id()));
    fs::create_dir_all(&directory).unwrap();
    let stock = stock_rustc::stock_rustc();
    for (index,source) in [
        "fn promoted()->&'static [i32;2]{&[7i32,9i32]}fn first()->&'static i32{&promoted()[0]}fn main(){println!(\"{}\",*first());}",
        "fn runtime()->i32{7i32}fn bad()->&'static [i32;0]{&[runtime();0]}fn main(){}",
        "fn bad()->&'static [i32;1]{&[1i32+2i32]}fn main(){}",
    ].into_iter().enumerate() {
        let input=directory.join(format!("source-{index}.rs"));
        fs::write(&input,source).unwrap();
        let result=Command::new(&stock).env_remove("RUSTC_BOOTSTRAP").args(["--edition=2024","--crate-type=bin"]).arg(&input).arg("-o").arg(directory.join(format!("stock-{index}"))).output().unwrap();
        assert!(result.status.success(),"stock-valid boundary: {}",String::from_utf8_lossy(&result.stderr));
        for workers in [1,8] { for check in [true,false] {
            let failure = justrust_engine::compiler::compile_observed(source.into(),workers,check).err().unwrap_or_else(||panic!("unproven scope accepted: {source}"));
            assert!(failure.to_string().contains("unsupported:"),"{failure}");
            assert!(failure.resource_usage.first_failure.is_none());
            let report=failure.report.as_ref().expect("captured unsupported graph");
            assert!(report.capture_complete);
            if check { assert!(report.node_evaluations.iter().all(|(key,_)| !matches!(key.kind,103|105|130|131|133))); }
        }}
    }
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn returned_promoted_arrays_match_stock() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check_with_compiler(
        "returned-promoted-arrays",
        &[
            Case {
                name: "original_returned_array_and_static",
                source: r#"static OFFSET: i32 = 5i32;
fn promoted() -> &'static [i32; 2] { &[7i32, 9i32] }
fn main() {
    let values = promoted();
    println!("{} {} {}", values[0], values[1], values[0] + OFFSET);
}
"#,
                expect: Expect::Run("7 9 12\n"),
            },
            Case {
                name: "repeated_calls_keep_static_storage_after_other_frames",
                source: r#"fn promoted()->&'static [i32;2]{&[7i32,9i32]}fn churn(x:i32)->i32{let a=[x,x+1,x+2];a[0]+a[1]+a[2]}fn main(){let a=promoted();println!("{}",churn(100));let b=promoted();println!("{} {} {} {}",a[0],a[1],b[0],b[1]);}"#,
                expect: Expect::Run("303\n7 9 7 9\n"),
            },
            Case {
                name: "nested_arrays_and_repeat_are_real_packed_memory",
                source: r#"fn nested()->&'static [[u16;2];2]{&[[257u16,513u16],[1025u16,2049u16]]}fn repeated()->&'static [[u16;2];3]{&[[5u16,9u16];3]}fn main(){let a=nested();let b=repeated();println!("{} {} {} {} {}",a[0][0],a[0][1],a[1][0],a[1][1],b[2][1]);}"#,
                expect: Expect::Run("257 513 1025 2049 9\n"),
            },
            Case {
                name: "zero_one_and_nested_empty_qualify_types_and_values",
                source: r#"fn empty()->&'static [i32;0]{&[]}fn one()->&'static [i32;1]{&[19i32]}fn nested()->&'static [[u8;0];2]{&[[],[]]}fn main(){let a=empty();let b=one();let c=nested();println!("{} {} {}",a.len(),b[0],c.len());}"#,
                expect: Expect::Run("0 19 2\n"),
            },
            Case {
                name: "dependent_scalar_constants_keep_graph_inputs",
                source: r#"const FIRST:i32=7i32;const SECOND:i32=FIRST+2i32;fn promoted()->&'static [i32;2]{&[FIRST,SECOND]}fn main(){let a=promoted();println!("{} {}",a[0],a[1]);}"#,
                expect: Expect::Run("7 9\n"),
            },
            Case {
                name: "reading_and_reborrowing_returned_static_elements",
                source: r#"fn promoted()->&'static [i32;2]{&[7i32,9i32]}fn first()->&'static i32{let values=promoted();&values[0]}fn read(a:&[i32;2])->i32{a[0]+a[1]}fn main(){println!("{} {}",*first(),read(promoted()));}"#,
                expect: Expect::Run("7 16\n"),
            },
            Case {
                name: "signed_narrow_wide_bool_and_character_arrays",
                source: r#"fn a()->&'static [i8;2]{&[-7i8,-9i8]}fn b()->&'static [i16;2]{&[-257i16,-513i16]}fn c()->&'static [i32;2]{&[-65537i32,-131073i32]}fn d()->&'static [i64;2]{&[-4294967297i64,4294967299i64]}fn e()->&'static [u128;2]{&[18446744073709551617u128,36893488147419103235u128]}fn flags()->&'static [bool;2]{&[true,false]}fn chars()->&'static [char;2]{&['a','🦀']}fn main(){let a=a();let b=b();let c=c();let d=d();let e=e();let f=flags();let g=chars();println!("{} {} {} {} {} {} {} {} {} {} {} {} {} {}",a[0],a[1],b[0],b[1],c[0],c[1],d[0],d[1],e[0],e[1],f[0],f[1],g[0],g[1]);}"#,
                expect: Expect::Run(
                    "-7 -9 -257 -513 -65537 -131073 -4294967297 4294967299 18446744073709551617 36893488147419103235 true false a 🦀\n",
                ),
            },
            Case {
                name: "immutable_static_read_is_not_constant_promotion",
                source: "static S:i32=7;fn bad()->&'static [i32;1]{&[S]}fn main(){}",
                expect: Expect::Invalid("E0515"),
            },
            Case {
                name: "imported_static_read_is_not_constant_promotion",
                source: "mod values{pub static S:i32=7;}use values::S as N;fn bad()->&'static [[i32;1];1]{&[[N]]}fn main(){}",
                expect: Expect::Invalid("E0515"),
            },
            Case {
                name: "unused_invalid_const_dependency_is_not_snapshotted",
                source: "const BAD:i32=1i32/0i32;fn promoted()->&'static [i32;1]{&[7i32]}fn main(){}",
                expect: Expect::Invalid("E0080"),
            },
            Case {
                name: "cyclic_const_dependency_is_not_snapshotted",
                source: "const A:i32=B;const B:i32=A;fn promoted()->&'static [i32;1]{&[A]}fn main(){}",
                expect: Expect::Invalid("E0391"),
            },
            Case {
                name: "unused_runtime_initializer_cannot_be_promoted",
                source: "fn runtime()->i32{7i32}fn bad()->&'static [i32;1]{&[runtime()]}fn main(){}",
                expect: Expect::Invalid("E0515"),
            },
            Case {
                name: "unused_owned_drop_array_cannot_be_promoted",
                source: "struct D{i:i32}impl Drop for D{fn drop(&mut self){}}fn bad()->&'static [D;1]{&[D{i:7i32}]}fn main(){}",
                expect: Expect::Invalid("E0515"),
            },
            Case {
                name: "unused_interior_mutable_array_cannot_be_promoted",
                source: "use core::cell::Cell;fn bad()->&'static [Cell<i32>;1]{&[Cell::new(7i32)]}fn main(){}",
                expect: Expect::Invalid("E0515"),
            },
            Case {
                name: "local_copy_is_not_a_constant_promotion",
                source: "fn bad()->&'static [i32;2]{let a=[7i32,9i32];&a}fn main(){}",
                expect: Expect::Invalid("E0515"),
            },
            Case {
                name: "zero_repeat_does_not_skip_operand_type_check",
                source: "fn bad()->&'static [i32;0]{&[true;0]}fn main(){}",
                expect: Expect::Invalid("E0308"),
            },
            Case {
                name: "unused_invalid_nested_element_is_checked",
                source: "fn bad()->&'static [[i32;1];1]{&[[true]]}fn main(){}",
                expect: Expect::Invalid("E0308"),
            },
        ],
        std::path::Path::new(&stock_rustc::stock_rustc()),
    );
}

#[test]
fn owned_aggregate_projected_borrows_match_stock() {
    closure_oracle::check_with_compiler(
        "owned-aggregate-projected-borrows",
        &[
            closure_oracle::Case {
                name: "constant_and_variable_receivers_and_indices",
                source: r#"struct Table { values: [i32; 3], sum: i32 }
const fn make_table() -> Table { let values=[7i32,11i32,24i32]; Table {values,sum:values[0]+values[1]+values[2]} }
const TABLE:Table=make_table();
fn main(){let table=make_table();let i=1usize;println!("{} {} {} {} {}",TABLE.values.len(),TABLE.values[0],TABLE.values[i],TABLE.values[2],TABLE.sum);println!("{} {} {} {} {}",table.values.len(),table.values[0],table.values[i],table.values[2],table.sum);}"#,
                expect: closure_oracle::Expect::Run("3 7 11 24 42\n3 7 11 24 42\n"),
            },
            closure_oracle::Case {
                name: "owned_call_roots_evaluate_once_in_argument_order",
                source: r#"struct Table{values:[i32;3],sum:i32}
fn make(n:i32)->Table{println!("make {}",n);Table{values:[n,n+1,n+2],sum:n+3}}
fn main(){println!("{} {} {}",make(1).values.len(),make(7).values[1],make(20).sum);}"#,
                expect: closure_oracle::Expect::Run("make 1\nmake 7\nmake 20\n3 8 23\n"),
            },
            closure_oracle::Case {
                name: "runtime_index_follows_owned_receiver_once",
                source: r#"struct Table{values:[i32;3]}fn make()->Table{println!("owner");Table{values:[7,11,24]}}fn index()->usize{println!("index");1usize}fn main(){println!("{}",make().values[index()]);}"#,
                expect: closure_oracle::Expect::Run("owner\nindex\n11\n"),
            },
            closure_oracle::Case {
                name: "non_copy_owner_lives_through_projected_borrow",
                source: r#"struct Table{values:[i32;3]}impl Drop for Table{fn drop(&mut self){println!("drop {}",self.values[0]);}}fn make()->Table{println!("owner");Table{values:[7,11,24]}}fn main(){println!("{}",make().values.len());println!("done");}"#,
                expect: closure_oracle::Expect::Run("owner\n3\ndrop 7\ndone\n"),
            },
            closure_oracle::Case {
                name: "extended_owned_projection_is_a_real_place",
                source: r#"struct Table{values:[i32;3]}const TABLE:Table=Table{values:[7,11,24]};fn make()->Table{Table{values:[7,11,24]}}fn main(){let r=&make().values[1];let m=&mut make().values;(*m)[0]=9;println!("{} {} {}",*r,m.len(),TABLE.values[0]);}"#,
                expect: closure_oracle::Expect::Run("11 3 7\n"),
            },
            closure_oracle::Case {
                name: "returned_reference_projection_keeps_original_storage",
                source: r#"struct Table{values:[i32;3]}fn get(t:&mut Table)->&mut Table{println!("get");t}fn main(){let mut t=Table{values:[7,11,24]};let r=&mut get(&mut t).values;(*r)[0]=9;println!("{} {}",r.len(),t.values[0]);}"#,
                expect: closure_oracle::Expect::Run("get\n3 9\n"),
            },
            closure_oracle::Case {
                name: "divergent_index_does_not_skip_checking_later_arguments",
                source: "struct Table{values:[i32;3]}fn make()->Table{Table{values:[7,11,24]}}fn main(){println!(\"{} {}\",make().values[{return;0usize}],true+1);}",
                expect: closure_oracle::Expect::Invalid("E0369"),
            },
            closure_oracle::Case {
                name: "unused_invalid_constant_is_not_bypassed",
                source: "struct Table{values:[i32;3]}const fn make()->Table{Table{values:[7,11,24]}}const TABLE:Table=make();const BAD:i32=1/0;fn main(){println!(\"{}\",TABLE.values.len());}",
                expect: closure_oracle::Expect::Invalid("E0080"),
            },
            closure_oracle::Case {
                name: "unused_invalid_body_is_not_bypassed",
                source: "struct Table{values:[i32;3]}const TABLE:Table=Table{values:[7,11,24]};fn bad(){let x:i32=true;}fn main(){println!(\"{}\",TABLE.values.len());}",
                expect: closure_oracle::Expect::Invalid("E0308"),
            },
            closure_oracle::Case {
                name: "owned_projection_cannot_escape_temporary_storage",
                source: "struct Table{values:[i32;3]}fn make()->Table{Table{values:[7,11,24]}}fn bad<'a>()->&'a [i32;3]{&make().values}fn main(){}",
                expect: closure_oracle::Expect::Invalid("E0515"),
            },
        ],
        std::path::Path::new(&stock_rustc::stock_rustc()),
    );
}

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
                name: "generic_source_scalar_operations_preserve_storage",
                source: r#"use core::cell::Cell;
fn update<T>(cell:&Cell<usize>,marker:T)->usize{let next=Cell::new(cell.get()+1usize);cell.set(next.get());cell.get()}
fn main(){let cell=Cell::new(3usize);println!("{} {}",update(&cell,1i32),cell.get());println!("{} {}",update(&cell,true),cell.get());}"#,
                expect: Expect::Run("4 4\n5 5\n"),
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
