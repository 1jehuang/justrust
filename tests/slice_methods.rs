//! Prelude slice methods (`swap`, `to_vec`, `contains`, `first`, `last`,
//! `is_empty`), `Vec::extend_from_slice` and Vec range views `&v[a..b]`.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#![allow(clippy::duplicate_mod)]
#[path = "support/oracle.rs"]
mod oracle;
#[path = "support/stock_rustc.rs"]
mod stock_rustc;
use oracle::{Case, Expect, check};
use std::process::Command;

#[test]
fn independent_mutable_slice_tuple_matches_stock() {
    #[path = "support/closure_oracle.rs"]
    mod closure_oracle;
    closure_oracle::check(
        "independent-mutable-slice-tuple",
        &[
            closure_oracle::Case {
                name: "separate_named_origins",
                source: "fn pair<'a,'b>(a:&'a mut[i32],b:&'b mut[i32])->(&'a mut[i32],&'b mut[i32]){(a,b)} fn main(){let mut a=[1i32,2];let mut b=[3i32,4];let (x,y)=pair(&mut a,&mut b);x[0]=10;y[1]=40;println!(\"{} {}\",x[0],y[1]);println!(\"{} {}\",a[0],b[1]);}",
                expect: closure_oracle::Expect::Run("10 40\n10 40\n"),
            },
            closure_oracle::Case {
                name: "overlapping_actuals",
                source: "fn pair<'a,'b>(a:&'a mut[i32],b:&'b mut[i32])->(&'a mut[i32],&'b mut[i32]){(a,b)} fn main(){let mut a=[1i32];let(x,y)=pair(&mut a,&mut a);x[0]=2;y[0]=3;}",
                expect: closure_oracle::Expect::Invalid("E0499"),
            },
            closure_oracle::Case {
                name: "duplicate_handle",
                source: "fn pair<'a,'b>(a:&'a mut[i32],b:&'b mut[i32])->(&'a mut[i32],&'b mut[i32]){(a,b)} fn main(){let mut a=[1i32];let s:&mut[i32]=&mut a;let(x,y)=pair(s,s);x[0]=2;y[0]=3;}",
                expect: closure_oracle::Expect::Invalid("E0499"),
            },
            closure_oracle::Case {
                name: "wrong_result_source",
                source: "fn pair<'a,'b>(a:&'a mut[i32],b:&'b mut[i32])->(&'a mut[i32],&'b mut[i32]){(b,a)} fn main(){}",
                expect: closure_oracle::Expect::Invalid("lifetime may not live long enough"),
            },
            closure_oracle::Case {
                name: "output_outlives_owner",
                source: "fn pair<'a,'b>(a:&'a mut[i32],b:&'b mut[i32])->(&'a mut[i32],&'b mut[i32]){(a,b)} fn main(){let mut a=[1i32];let escaped;{let mut b=[2i32];let(x,y)=pair(&mut a,&mut b);x[0]=3;escaped=y;}escaped[0]=4;}",
                expect: closure_oracle::Expect::Invalid("E0597"),
            },
        ],
    );
}

#[test]
fn generic_independent_mutable_slice_tuple_matches_stock() {
    #[path = "support/closure_oracle.rs"]
    mod closure_oracle;
    closure_oracle::check(
        "generic-independent-mutable-slice-tuple",
        &[closure_oracle::Case {
            name: "generic_empty_and_reverse_mapping",
            source: "fn pair<'a,'b,T>(a:&'a mut[T],b:&'b mut[T])->(&'b mut[T],&'a mut[T]){(b,a)} fn main(){let mut a:[u64;0]=[];let mut b=[3u64];let (x,y)=pair(&mut a,&mut b);x[0]=9;println!(\"{} {}\",x[0],y.len());}",
            expect: closure_oracle::Expect::Run("9 0\n"),
        }],
    );
}

#[test]
fn aggregate_mutable_slice_carriers_match_stock() {
    #[path = "support/closure_oracle.rs"]
    mod closure_oracle;
    closure_oracle::check(
        "aggregate-mutable-slice-carriers",
        &[
            closure_oracle::Case {
                name: "optional_split_descriptor",
                source: "fn split(v:&mut[i32])->Option<(&mut i32,&mut[i32])>{v.split_first_mut()} fn main(){let mut a=[5i32,1,4];if let Some((h,t))=split(&mut a){*h+=t[0]+t[1];t[0]=9;}println!(\"{:?}\",a);let mut e:[i32;0]=[];println!(\"{}\",split(&mut e).is_none());}",
                expect: closure_oracle::Expect::Run("[10, 9, 4]\ntrue\n"),
            },
            closure_oracle::Case {
                name: "nominal_independent_origins",
                source: "struct Views<'a,'b>{a:Option<&'a mut[i32]>,b:(&'b mut[i32],usize)}fn views<'a,'b>(a:&'a mut[i32],b:&'b mut[i32])->Views<'a,'b>{Views{a:Some(a),b:(b,7)}}fn main(){let mut a=[1i32];let mut b=[2i32];let n;{let v=views(&mut a,&mut b);if let Some(x)=v.a{x[0]=10;}v.b.0[0]=20;n=v.b.1;}println!(\"{} {} {}\",a[0],b[0],n);}",
                expect: closure_oracle::Expect::Run("10 20 7\n"),
            },
            closure_oracle::Case {
                name: "exclusive_enum_alternatives_reuse_one_input",
                source: "enum Pick<'a>{A(&'a mut[i32]),B(&'a mut[i32])}fn pick<'a>(x:&'a mut[i32],first:bool)->Pick<'a>{if first{Pick::A(x)}else{Pick::B(x)}}fn main(){let mut a=[1i32];for flag in [true,false]{match pick(&mut a,flag){Pick::A(x)=>x[0]+=2,Pick::B(x)=>x[0]+=3}}println!(\"{}\",a[0]);}",
                expect: closure_oracle::Expect::Run("6\n"),
            },
        ],
    );
}

#[test]
fn by_value_reference_self_autoref_receivers_match_stock() {
    #[path = "support/closure_oracle.rs"]
    mod closure_oracle;
    closure_oracle::check(
        "by-value-reference-self-autoref",
        &[
            closure_oracle::Case {
                name: "shared_slice_methods_on_mutable_slice",
                source: "fn split_first_mut(v:&mut[i32])->Option<(&mut i32,&mut[i32])>{v.split_first_mut()} fn main(){let mut nums=vec![3,1,4,1,5];if let Some((h,t))=split_first_mut(&mut nums){*h+=t.iter().sum::<i32>();t[0]=9;}println!(\"{:?}\",nums);let t:&mut[i32]=&mut nums[1..];for x in t.iter(){print!(\"{} \",x);}println!(\"{}\",t.iter().map(|x|x*2).count());}",
                expect: closure_oracle::Expect::Run("[14, 9, 4, 1, 5]\n9 4 1 5 4\n"),
            },
            closure_oracle::Case {
                name: "user_reference_self_impls",
                source: "trait Tot{fn tot(self)->usize;}impl Tot for &String{fn tot(self)->usize{self.len()}}impl Tot for &mut Vec<i32>{fn tot(self)->usize{self.push(1);self.len()}}impl Tot for &i32{fn tot(self)->usize{*self as usize}}fn main(){let s=String::from(\"abc\");println!(\"{}\",s.tot());let mut v=vec![1];println!(\"{}\",v.tot());let r=&mut v;println!(\"{}\",r.tot());println!(\"{:?}\",v);let mut n=5i32;let m=&mut n;println!(\"{}\",m.tot());}",
                expect: closure_oracle::Expect::Run("3\n2\n3\n[1, 1, 1]\n5\n"),
            },
            closure_oracle::Case {
                name: "vector_split_at_mut_through_deref_mut",
                source: "trait Bump{fn bump(self)->usize;}impl Bump for &mut [i32]{fn bump(self)->usize{self[0]+=1;self.len()}}fn main(){let mut grid=vec![0usize,0,0,0,0,7];let(top,bottom)=grid.split_at_mut(3);top[0]=bottom[2]*10;println!(\"{} {}\",top.len(),bottom.len());println!(\"{:?}\",grid);let mut v=vec![1,2];let r:&mut Vec<i32>=&mut v;println!(\"{}\",r.bump());println!(\"{:?}\",v);}",
                expect: closure_oracle::Expect::Run("3 3\n[70, 0, 0, 0, 0, 7]\n2\n[2, 2]\n"),
            },
            closure_oracle::Case {
                name: "unused_shared_autoref_conflicts_with_mutable_loan",
                source: "fn bad(t:&mut[i32]){let i=t.iter();t[0]=1;let _=i.count();}fn main(){}",
                expect: closure_oracle::Expect::Invalid("E0506"),
            },
            closure_oracle::Case {
                name: "unused_mutable_reference_self_through_shared",
                source: "trait Bump{fn bump(self);}impl Bump for &mut Vec<i32>{fn bump(self){self.push(1);}}fn bad(v:&Vec<i32>){v.bump();}fn main(){}",
                expect: closure_oracle::Expect::Invalid("E0596"),
            },
            closure_oracle::Case {
                name: "split_at_mut_rows_of_vectors",
                source: "fn main(){let mut grid=vec![vec![0usize,1,2],vec![3,4,5],vec![6,7,8]];let(top,bottom)=grid.split_at_mut(1);top[0][0]=bottom[0][2]*10;bottom[1].push(9);let(a,b)=bottom.split_at_mut(1);a[0][1]+=b[0][3];println!(\"{:?}\",grid);let mut w=vec![String::from(\"a\"),String::from(\"b\")];if let Some((h,t))=w.split_first_mut(){h.push_str(&t[0]);}println!(\"{:?}\",w);}",
                expect: closure_oracle::Expect::Run(
                    "[[50, 1, 2], [3, 13, 5], [6, 7, 8, 9]]\n[\"ab\", \"b\"]\n",
                ),
            },
            closure_oracle::Case {
                name: "unused_split_rows_overlap_owner",
                source: "fn bad(){let mut g=vec![vec![1usize]];let(a,_b)=g.split_at_mut(1);g.push(vec![]);a[0][0]=2;}fn main(){}",
                expect: closure_oracle::Expect::Invalid("E0499"),
            },
            closure_oracle::Case {
                name: "unused_overlapping_split_halves_used_together",
                source: "fn bad(){let mut g=vec![vec![1usize],vec![2]];let(a,_b)=g.split_at_mut(1);let(c,_d)=g.split_at_mut(1);a[0][0]=c[0][0];}fn main(){}",
                expect: closure_oracle::Expect::Invalid("E0499"),
            },
            closure_oracle::Case {
                name: "unused_split_at_mut_halves_overlap_owner",
                source: "fn bad(){let mut v=vec![1,2];let(a,_b)=v.split_at_mut(1);v.push(3);a[0]=2;}fn main(){}",
                expect: closure_oracle::Expect::Invalid("E0499"),
            },
        ],
    );
}

#[test]
fn aggregate_mutable_slice_carriers_reject_invalid_sources() {
    #[path = "support/closure_oracle.rs"]
    mod closure_oracle;
    closure_oracle::check(
        "aggregate-mutable-slice-invalid",
        &[
            closure_oracle::Case {
                name: "unused_same_variant_mutable_shared_alias",
                source: "enum Views<'a>{Both(&'a mut[i32],&'a[i32]),None}fn bad<'a>(x:&'a mut[i32])->Views<'a>{Views::Both(&mut*x,&*x)}fn main(){}",
                expect: closure_oracle::Expect::Invalid("E0502"),
            },
            closure_oracle::Case {
                name: "unused_duplicate_mutable_descriptor",
                source: "fn duplicate<'a>(x:&'a mut[i32])->Option<(&'a mut[i32],&'a mut[i32])>{Some((x,x))}fn main(){}",
                expect: closure_oracle::Expect::Invalid("E0499"),
            },
            closure_oracle::Case {
                name: "unused_wrong_named_origin",
                source: "fn swapped<'a,'b>(a:&'a mut[i32],b:&'b mut[i32])->Option<(&'a mut[i32],&'b mut[i32])>{Some((b,a))}fn main(){}",
                expect: closure_oracle::Expect::Invalid("lifetime may not live long enough"),
            },
            closure_oracle::Case {
                name: "optional_output_outlives_owner",
                source: "fn wrap(x:&mut[i32])->Option<&mut[i32]>{Some(x)}fn main(){let out;{let mut a=[1i32];out=wrap(&mut a);}if let Some(x)=out{x[0]=2;}}",
                expect: closure_oracle::Expect::Invalid("E0597"),
            },
        ],
    );
}

#[test]
fn mutable_slice_tuple_reference_elements_remain_unsupported() {
    let source = "fn pair<'a,'b,T>(a:&'a mut[T],b:&'b mut[T])->(&'a mut[T],&'b mut[T]){(a,b)}fn main(){let value=1i32;let mut a=[&value];let mut b=[&value];let(x,y)=pair(&mut a,&mut b);let _=x[0];let _=y[0];}";
    let root = scratch("tuple-reference-elements");
    let path = root.join("input.rs");
    std::fs::write(&path, source).unwrap();
    let stock = Command::new(stock_rustc::stock_rustc())
        .args(["--edition=2024", "-Cpanic=abort"])
        .arg(&path)
        .arg("-o")
        .arg(root.join("stock"))
        .output()
        .unwrap();
    assert!(stock.status.success(), "{stock:?}");
    for workers in [1, 8] {
        for check_only in [true, false] {
            let error = justrust_engine::compiler::compile(source.into(), workers, check_only)
                .err()
                .expect("mutable slices with reference elements must remain unsupported")
                .to_string();
            assert!(
                error.contains("unsupported")
                    && error.contains("slice")
                    && error.contains("reference"),
                "{error}"
            );
        }
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn slice_data_pointer_primitive_matches_stock() {
    use std::os::unix::fs::PermissionsExt;
    let source = r#"
fn pointer<T>(s:&mut[T])->*mut T{s.as_mut_ptr()}
fn main(){
    let mut a=[1i32,2,3];let p=pointer(&mut a);
    unsafe{*p.add(2)=9;}println!("{} {}",a[0],a[2]);
    let mut empty:[u64;0]=[];let p=pointer(&mut empty);
    println!("{} {}",p as usize!=0,(p as usize)%8==0);
    let mut z=[();3];let p=pointer(&mut z);unsafe{*p.add(2)=();}
    println!("{} {}",p as usize!=0,z.len());
}"#;
    let root = scratch("data-pointer-primitive");
    let stock_source = root.join("stock.rs");
    let stock_image = root.join("stock");
    std::fs::write(&stock_source, source).unwrap();
    let stock = Command::new(stock_rustc::stock_rustc())
        .args(["--edition=2024", "-Cpanic=abort"])
        .arg(&stock_source)
        .arg("-o")
        .arg(&stock_image)
        .output()
        .unwrap();
    assert!(stock.status.success(), "{stock:?}");
    let expected = Command::new(&stock_image).output().unwrap();
    assert!(expected.status.success());
    let native_source = source.replace("s.as_mut_ptr()", "__justrust_slice_as_mut_ptr!(s)");
    for workers in [1, 8] {
        for check in [true, false] {
            let result =
                justrust_engine::compiler::compile(native_source.clone(), workers, check).unwrap();
            if check {
                assert!(result.executable.is_none());
                assert!(
                    result
                        .report
                        .node_evaluations
                        .iter()
                        .all(|(key, _)| { !matches!(key.kind, 103 | 105 | 130 | 131 | 133) })
                );
            } else {
                let image = root.join(format!("native-{workers}"));
                std::fs::write(&image, result.executable.unwrap()).unwrap();
                std::fs::set_permissions(&image, std::fs::Permissions::from_mode(0o700)).unwrap();
                let actual = Command::new(&image).output().unwrap();
                assert!(actual.status.success(), "{actual:?}");
                assert_eq!(actual.stdout, expected.stdout);
            }
        }
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn associated_raw_pointer_method_preserves_source_identity() {
    #[path = "support/closure_oracle.rs"]
    mod closure_oracle;
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "associated-raw-pointer-method",
        &[
            Case {
                name: "owner_normalizes_exact_associated_pointee",
                source: "trait Data{type Element;fn data(&mut self)->*mut Self::Element;}struct Value{n:i32}impl Data for Value{type Element=i32;fn data(&mut self)->*mut i32{&mut self.n as *mut i32}}fn main(){let mut v=Value{n:4};let p=v.data();unsafe{*p=9;}println!(\"{}\",v.n);}",
                expect: Expect::Run("9\n"),
            },
            Case {
                name: "wrong_associated_pointer_kind_rejected_unused",
                source: "trait Data{type Element;fn data(&mut self)->*mut Self::Element;}struct Value;impl Data for Value{type Element=i32;fn data(&mut self)->*const i32{0usize as *const i32}}fn main(){}",
                expect: Expect::Invalid("E0053"),
            },
            Case {
                name: "wrong_associated_pointee_rejected_unused",
                source: "trait Data{type Element;fn data(&mut self)->*mut Self::Element;}struct Value;impl Data for Value{type Element=i32;fn data(&mut self)->*mut u32{0usize as *mut u32}}fn main(){}",
                expect: Expect::Invalid("E0053"),
            },
        ],
    );
}

#[test]
fn slice_mut_pointer_matches_stock() {
    #[path = "support/closure_oracle.rs"]
    mod closure_oracle;
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "slice-mut-pointer",
        &[Case {
            name: "generic_reborrow_and_whole_allocation",
            source: r#"
fn pointer<T>(s: &mut [T]) -> *mut T { s.as_mut_ptr() }
fn main() {
    let mut a = [1i32, 2, 3];
    let s: &mut [i32] = &mut a;
    let p = s.as_mut_ptr();
    let q = s.as_mut_ptr();
    unsafe { *p.add(2) = 30; *q = 10; }
    println!("{} {} {}", s[0], s[1], s[2]);
    let p = pointer(s);
    println!("{}", s.len());
    unsafe { *p.add(1) = 20; }
    println!("{}", s[1]);
    let mut b = [7u8, 8];
    let p = pointer(&mut b);
    unsafe { *p.add(1) = 9; }
    println!("{}", b[1]);
}
"#,
            expect: Expect::Run("10 2 30\n3\n20\n9\n"),
        }],
    );
}

#[test]
fn slice_mut_pointer_empty_zst_and_noncopy() {
    #[path = "support/closure_oracle.rs"]
    mod closure_oracle;
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "slice-mut-pointer-concrete",
        &[Case {
            name: "empty_zst_and_noncopy",
            source: r#"
struct Value { n: i32 }
fn main() {
    let mut empty: [u64; 0] = [];
    let e: &mut [u64] = &mut empty;
    let p = e.as_mut_ptr();
    println!("{} {}", p as usize != 0, (p as usize) % 8 == 0);
    let mut units = [(); 3];
    let s: &mut [()] = &mut units;
    let p = s.as_mut_ptr();
    unsafe { *p.add(2) = (); }
    println!("{} {}", p as usize != 0, s.len());
    let mut values = [Value { n: 4 }, Value { n: 5 }];
    let s: &mut [Value] = &mut values;
    let p = s.as_mut_ptr();
    unsafe { (*p.add(1)).n = 6; }
    let value = &s[1];
    println!("{}", value.n);
}
"#,
            expect: Expect::Run("true true\ntrue 3\n6\n"),
        }],
    );
}

#[test]
fn slice_mut_pointer_generic_aggregate_layout_matches_stock() {
    #[path = "support/closure_oracle.rs"]
    mod closure_oracle;
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "slice-mut-pointer-aggregate-layout",
        &[Case {
            name: "generic_nominal_nested_arrays_and_nominal_zsts",
            source: r#"
struct Value<T> { n: T }
struct Empty;
struct Zero { unit: (), empty: Empty }
struct AlignedZero { zero: [u128; 0] }
struct Padded<T> { before: [u128; 0], n: T, after: () }
fn advance<T>(s: &mut [T], n: usize) -> *mut T {
    unsafe { s.as_mut_ptr().add(n) }
}
fn replace<T>(s: &mut [T], n: usize, value: T) {
    unsafe { *advance(s, n) = value; }
}
fn main() {
    let mut values = [Value { n: 4u16 }, Value { n: 5u16 }];
    let p = advance(&mut values, 1);
    let base = advance(&mut values, 0);
    println!("{}", p as usize - base as usize);
    unsafe { (*p).n = 9; }
    replace(&mut values, 0, Value { n: 7u16 });
    println!("{} {}", values[0].n, values[1].n);
    let mut rows = [[Value { n: 10i64 }], [Value { n: 20i64 }]];
    replace(&mut rows, 1, [Value { n: 30i64 }]);
    println!("{} {}", rows[0][0].n, rows[1][0].n);
    let mut empty: [Value<u32>; 0] = [];
    let p = advance(&mut empty, 0);
    println!("{}", p as usize != 0);
    let mut zsts = [Empty, Empty];
    let p = advance(&mut zsts, 0);
    let q = advance(&mut zsts, 1);
    unsafe { *q = Empty; }
    println!("{} {}", p as usize != 0, p as usize == q as usize);
    let mut nested = [Zero { unit: (), empty: Empty }, Zero { unit: (), empty: Empty }];
    let p = advance(&mut nested, 0);
    let q = advance(&mut nested, 1);
    unsafe { *q = Zero { unit: (), empty: Empty }; }
    println!("{} {}", p as usize != 0, p as usize == q as usize);
    let mut aligned = [AlignedZero { zero: [] }, AlignedZero { zero: [] }];
    let p = advance(&mut aligned, 0);
    let q = advance(&mut aligned, 1);
    unsafe { *q = AlignedZero { zero: [] }; }
    println!("{} {}", p as usize % 16 == 0, p as usize == q as usize);
    let mut padded = [Padded { before: [], n: 3u8, after: () }, Padded { before: [], n: 4u8, after: () }];
    let p = advance(&mut padded, 0);
    let q = advance(&mut padded, 1);
    unsafe { (*q).n = 8; }
    println!("{} {} {}", q as usize - p as usize, p as usize % 16 == 0, padded[1].n);
}
"#,
            expect: Expect::Run(
                "2\n7 9\n10 30\ntrue\ntrue true\ntrue true\ntrue true\n16 true 8\n",
            ),
        }],
    );
}

#[test]
fn slice_mut_pointer_public_field_offsets_match_stock() {
    #[path = "support/closure_oracle.rs"]
    mod closure_oracle;
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "slice-mut-pointer-public-offsets",
        &[Case {
            name: "aligned_owning_bases_and_unambiguous_zst_addresses",
            source: r#"
struct Before<T> { z: [u128; 0], n: T, tail: () }
#[repr(C)] struct CBefore { z: [u128; 0], n: u8, tail: () }
fn identity(v: Before<u8>) -> Before<u8> { v }
fn observe(v: &Before<u8>) {
    let base = v as *const Before<u8> as usize;
    println!("{} {} {} {}", base % 16 == 0,
        &v.z as *const [u128; 0] as usize - base,
        &v.n as *const u8 as usize - base,
        &v.tail as *const () as usize - base);
}
fn main() {
    let v = Before { z: [], n: 3u8, tail: () };
    observe(&v);
    let v = identity(v);
    observe(&v);
    let mut a = [Before { z: [], n: 4u8, tail: () }, Before { z: [], n: 5u8, tail: () }];
    let p = a.as_mut_ptr();
    unsafe { observe(&*p); observe(&*p.add(1)); }
    let v = CBefore { z: [], n: 6u8, tail: () };
    let base = &v as *const CBefore as usize;
    println!("{} {} {} {}", base % 16 == 0,
        &v.z as *const [u128; 0] as usize - base,
        &v.n as *const u8 as usize - base,
        &v.tail as *const () as usize - base);
}
"#,
            expect: Expect::Run("true 0 0 1\ntrue 0 0 1\ntrue 0 0 1\ntrue 0 0 1\ntrue 0 0 1\n"),
        }],
    );
}

#[test]
fn slice_mut_pointer_unproved_physical_layout_is_rejected() {
    for (name, source, diagnostic) in [
        (
            "ambiguous_trailing_high_alignment_zst_address",
            "struct V{n:u8,z:[u128;0]}fn main(){let v=V{n:1,z:[]};let p=&v.z as *const [u128;0];println!(\"{}\",p as usize);}",
            "ZST field address",
        ),
        (
            "mutable_nested_trailing_high_alignment_zst_address",
            "struct V{n:u8,z:[u128;0]}struct Outer{value:V}fn pointer(s:&mut[Outer])->*mut[u128;0]{&mut s[0].value.z as *mut[u128;0]}fn main(){let mut a=[Outer{value:V{n:1,z:[]}}];let p=pointer(&mut a);println!(\"{}\",p as usize);}",
            "ZST field address",
        ),
        (
            "ambiguous_leading_low_alignment_zst_address",
            "struct V{z:(),n:u32}fn main(){let v=V{z:(),n:1};let p=&v.z as *const ();println!(\"{}\",p as usize);}",
            "ZST field address",
        ),
        (
            "multiple_storage_fields",
            "struct V{a:i32,b:i32}fn main(){let mut a=[V{a:1,b:2},V{a:3,b:4}];let p=a.as_mut_ptr();unsafe{let _=p.add(1);}}",
            "public target memory representation",
        ),
        (
            "niche_enum",
            "enum V{None,Some(i32)}fn main(){let mut a=[V::None,V::Some(1)];let p=a.as_mut_ptr();unsafe{let _=p.add(1);}}",
            "target aggregate size",
        ),
        (
            "explicit_alignment",
            "#[repr(align(16))]struct V{n:u32}fn main(){let mut a=[V{n:1},V{n:2}];let p=a.as_mut_ptr();unsafe{let _=p.add(1);}}",
            "repr(align) layout is not implemented",
        ),
        (
            "packed_layout",
            "#[repr(packed)]struct V{n:u32}fn main(){let mut a=[V{n:1},V{n:2}];let p=a.as_mut_ptr();unsafe{let _=p.add(1);}}",
            "repr(C)/repr(align(N)) structs",
        ),
    ] {
        let root = scratch(&format!("pointer-layout-{name}"));
        let path = root.join("main.rs");
        std::fs::write(&path, source).unwrap();
        let stock = Command::new(stock_rustc::stock_rustc())
            .args(["--edition=2024", "--emit=metadata"])
            .arg(&path)
            .arg("-o")
            .arg(root.join("stock.rmeta"))
            .output()
            .unwrap();
        assert!(stock.status.success(), "{name}: {stock:?}");
        for workers in ["1", "8"] {
            for check in [true, false] {
                let image = root.join(format!("native-{workers}-{check}"));
                let mut command = Command::new(env!("CARGO_BIN_EXE_jr-rust"));
                command.arg(&path).args(["--workers", workers]);
                if check {
                    command.arg("--check");
                } else {
                    command.arg("-o").arg(&image);
                }
                let result = command.output().unwrap();
                let error = String::from_utf8_lossy(&result.stderr);
                assert!(
                    !result.status.success(),
                    "{name}: unsupported layout compiled"
                );
                assert!(
                    error.contains("unsupported") && error.contains(diagnostic),
                    "{name}: {error}"
                );
                assert!(
                    !image.exists(),
                    "{name}: unsupported layout emitted an image"
                );
            }
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn slice_mut_pointer_empty_vec() {
    #[path = "support/closure_oracle.rs"]
    mod closure_oracle;
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "slice-mut-pointer-empty-vec",
        &[Case {
            name: "empty_heap_storage_is_nonnull_and_aligned",
            source: "fn main(){let p=Vec::<u64>::new().as_mut_slice().as_mut_ptr();println!(\"{} {}\",p as usize!=0,(p as usize)%8==0);}",
            expect: Expect::Run("true true\n"),
        }],
    );
}

#[test]
fn slice_mut_pointer_invalid_boundaries() {
    #[path = "support/closure_oracle.rs"]
    mod closure_oracle;
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "slice-mut-pointer-invalid",
        &[
            Case {
                name: "shared_receiver",
                source: "fn bad(s:&[i32]){let _=s.as_mut_ptr();}fn main(){}",
                expect: Expect::Invalid("E0596"),
            },
            Case {
                name: "dereference_requires_unsafe",
                source: "fn bad(s:&mut[i32]){let p=s.as_mut_ptr();*p=2;}fn main(){}",
                expect: Expect::Invalid("E0133"),
            },
            Case {
                name: "generic_shared_receiver",
                source: "fn bad<T>(s:&[T]){let _=s.as_mut_ptr();}fn main(){}",
                expect: Expect::Invalid("E0596"),
            },
            Case {
                name: "generic_add_requires_unsafe",
                source: "fn bad<T>(s:&mut[T]){let _=s.as_mut_ptr().add(1);}fn main(){}",
                expect: Expect::Invalid("E0133"),
            },
            Case {
                name: "raw_call_reborrow_requires_unsafe",
                source: "fn identity<T>(p:*mut T)->*mut T{p}fn bad<T>(p:*mut T){let _=&*identity(p);}fn main(){}",
                expect: Expect::Invalid("E0133"),
            },
            Case {
                name: "raw_const_call_reborrow_cannot_mutate",
                source: "fn identity<T>(p:*const T)->*const T{p}fn bad<T>(p:*const T){unsafe{let _=&mut *identity(p);}}fn main(){}",
                expect: Expect::Invalid("cannot borrow as mutable"),
            },
            Case {
                name: "generic_wrong_element_type",
                source: "fn bad<T>(s:&mut[T])->*mut u8{s.as_mut_ptr()}fn main(){}",
                expect: Expect::Invalid("E0308"),
            },
            Case {
                name: "wrong_element_type",
                source: "fn bad(s:&mut[i32]){let _: *mut u8=s.as_mut_ptr();}fn main(){}",
                expect: Expect::Invalid("E0308"),
            },
        ],
    );
}

#[test]
fn heap_stored_slice_carriers_match_stock() {
    #[path = "support/closure_oracle.rs"]
    mod closure_oracle;
    closure_oracle::check(
        "heap-stored-slice-carriers",
        &[
            closure_oracle::Case {
                name: "chunks_collect",
                source: "fn main(){let a=[1i32,2];let v:Vec<&[i32]>=a.chunks(1).collect();let x=v[0][0];let y=v[1][0];println!(\"{} {} {}\",v.len(),x,y);}",
                expect: closure_oracle::Expect::Run("2 1 2\n"),
            },
            closure_oracle::Case {
                name: "flat_map_copied",
                source: "fn main(){let v:Vec<Vec<i32>>=vec![vec![1,2],vec![3]];let f:Vec<i32>=v.iter().flat_map(|w|w.iter().copied()).collect();let n=v.iter().flat_map(|w|w.iter().copied()).count();println!(\"{:?} {}\",f,n);}",
                expect: closure_oracle::Expect::Run("[1, 2, 3] 3\n"),
            },
            closure_oracle::Case {
                name: "option_iter_take_and_vec_of_slices",
                source: "fn main(){let a=vec![1i32,2,3];let mut v:Vec<&[i32]>=Vec::new();v.push(&a[..1]);v.push(&a[1..]);let mut o=Some(a.iter());let n=o.take().map(|i|i.count());println!(\"{} {:?} {}\",v.len(),n,o.is_none());}",
                expect: closure_oracle::Expect::Run("2 Some(3) true\n"),
            },
            closure_oracle::Case {
                name: "collected_chunks_escape",
                source: "fn main(){let v:Vec<&[i32]>;{let a=[1i32,2];v=a.chunks(1).collect();}println!(\"{}\",v.len());}",
                expect: closure_oracle::Expect::Invalid("E0597"),
            },
            closure_oracle::Case {
                name: "flat_map_source_moved",
                source: "fn main(){let f:Vec<i32>;{let v:Vec<Vec<i32>>=vec![vec![1,2],vec![3]];let it=v.iter().flat_map(|w|w.iter().copied());drop(v);f=it.collect();}println!(\"{:?}\",f);}",
                expect: closure_oracle::Expect::Invalid("E0505"),
            },
            closure_oracle::Case {
                name: "flat_map_source_mutated",
                source: "fn main(){let mut v:Vec<Vec<i32>>=vec![vec![1,2],vec![3]];let it=v.iter().flat_map(|w|w.iter().copied());v.push(vec![4]);let f:Vec<i32>=it.collect();println!(\"{:?}\",f);}",
                expect: closure_oracle::Expect::Invalid("E0502"),
            },
            closure_oracle::Case {
                name: "pushed_slice_outlives_owner",
                source: "fn main(){let mut v:Vec<&[i32]>=Vec::new();{let a=vec![1,2];v.push(&a[..]);}println!(\"{}\",v.len());}",
                expect: closure_oracle::Expect::Invalid("E0597"),
            },
        ],
    );
}

const POSITIVE: &str = r#"
fn f(v: &mut [i32]) { v.swap(0, 1); }
fn merge(v: &[i32]) -> Vec<i32> {
    if v.len() <= 1 { return v.to_vec(); }
    let mid = v.len() / 2;
    let mut out = Vec::new();
    out.extend_from_slice(&v[mid..]);
    out.extend_from_slice(&v[..mid]);
    out
}
fn main() {
    let mut a = vec![1, 2, 3, 4];
    f(a.as_mut_slice());
    let s: &[i32] = a.as_slice();
    let b = s.to_vec();
    let c = s.contains(&2);
    let d = s.contains(&9);
    let e = s.is_empty();
    let fi = s.first();
    let la = s.last();
    let x = b[0];
    println!("{} {} {} {} {:?} {:?}", x, c, d, e, fi, la);
    let t = &a[1..3];
    let n = t.len();
    let u = &a[..2];
    let m = u.to_vec();
    let k = m[1];
    let r = merge(&a[0..4]);
    let r0 = r[0];
    let empty: &[i32] = &a[4..];
    let z = empty.first();
    println!("{} {} {} {} {:?}", n, k, r0, empty.is_empty(), z);
}
"#;
const POSITIVE_OUT: &str = "2 true false false Some(2) Some(4)\n2 1 3 true None\n";

#[test]
fn shared_slice_carriers_match_stock() {
    #[path = "support/closure_oracle.rs"]
    mod closure_oracle;
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "shared-slice-carriers",
        &[Case {
            name: "reference_tuple_array_and_generic_identity",
            source: r#"
fn indirect(s: &&[u8]) -> usize { (*s).len() }
fn tuple(s: (&[u8], u8)) -> usize { s.0.len() + s.1 as usize }
fn array(s: [&[u8]; 1]) -> usize { s[0].len() }
fn id<T>(x: T) -> T { x }
fn main() {
    let a = [1u8, 2, 3];
    let s: &[u8] = &a;
    let t = id(s);
    println!("{} {} {} {}", indirect(&s), tuple((s, 2)), array([s]), t.len());
}
"#,
            expect: Expect::Run("3 5 3 3\n"),
        }],
    );
}

#[test]
fn slice_methods_match_stock() {
    check(
        "slice-methods",
        &[
            Case {
                name: "methods",
                source: POSITIVE,
                expect: Expect::Run(POSITIVE_OUT),
            },
            Case {
                name: "shared_swap_unused",
                source: "fn bad(s: &[i32]) { s.swap(0, 1); }\nfn main() { println!(\"x\"); }\n",
                expect: Expect::Invalid("E0596"),
            },
            Case {
                name: "shared_range_mut_unused",
                source: "fn bad(v: &Vec<i32>) { let s = &mut v[1..]; s[0] = 1; }\nfn main() { println!(\"x\"); }\n",
                expect: Expect::Invalid("E0596"),
            },
        ],
    );
}

fn scratch(tag: &str) -> std::path::PathBuf {
    let p = std::env::var_os("JCODE_SCRATCH_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join(format!("jr-slice-methods-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

#[test]
fn workers_one_and_eight_check_build_run() {
    for workers in ["1", "8"] {
        let dir = scratch(workers);
        let src = dir.join("main.rs");
        std::fs::write(&src, POSITIVE).unwrap();
        let check = Command::new(env!("CARGO_BIN_EXE_jr-rust"))
            .arg(&src)
            .args(["--check", "--workers", workers])
            .output()
            .unwrap();
        assert!(
            check.status.success(),
            "{}",
            String::from_utf8_lossy(&check.stderr)
        );
        let image = dir.join("native");
        let build = Command::new(env!("CARGO_BIN_EXE_jr-rust"))
            .arg(&src)
            .arg("-o")
            .arg(&image)
            .args(["--workers", workers])
            .output()
            .unwrap();
        assert!(
            build.status.success(),
            "{}",
            String::from_utf8_lossy(&build.stderr)
        );
        let ran = Command::new(&image).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&ran.stdout), POSITIVE_OUT);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// Out-of-bounds `swap` panics with stock's index message under panic=abort.
#[test]
fn swap_out_of_bounds_panics_like_stock() {
    let source = "fn f(v: &mut [i32]) { v.swap(0, 5); }\nfn main() { let mut a = vec![1, 2, 3]; println!(\"before\"); f(a.as_mut_slice()); println!(\"after\"); }\n";
    let dir = scratch("panic");
    let src = dir.join("main.rs");
    std::fs::write(&src, source).unwrap();
    let image = dir.join("native");
    let build = Command::new(env!("CARGO_BIN_EXE_jr-rust"))
        .arg(&src)
        .arg("-o")
        .arg(&image)
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let native = Command::new(&image).output().unwrap();
    let stock_image = dir.join("stock");
    let stock = Command::new(stock_rustc::stock_rustc())
        .env_remove("RUSTC_BOOTSTRAP")
        .args(["--edition=2024", "-Cpanic=abort", "-Awarnings"])
        .arg(&src)
        .arg("-o")
        .arg(&stock_image)
        .output()
        .unwrap();
    assert!(stock.status.success());
    let stock = Command::new(&stock_image).output().unwrap();
    assert_eq!(native.stdout, stock.stdout);
    assert_eq!(native.status.code(), stock.status.code());
    let message = "index out of bounds: the len is 3 but the index is 5";
    assert!(String::from_utf8_lossy(&native.stderr).contains(message));
    assert!(String::from_utf8_lossy(&stock.stderr).contains(message));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn split_at_out_of_bounds_panics_like_stock() {
    let dir = scratch("split-panic");
    let src = dir.join("main.rs");
    std::fs::write(&src, "fn f(v:&[i32]){let _=v.split_at(4usize);}fn main(){println!(\"before\");f(&[1i32,2i32,3i32]);println!(\"after\");}").unwrap();
    let stock_image = dir.join("stock");
    let stock = Command::new(stock_rustc::stock_rustc())
        .env_remove("RUSTC_BOOTSTRAP")
        .args(["--edition=2024", "-Cpanic=abort", "-Awarnings"])
        .arg(&src)
        .arg("-o")
        .arg(&stock_image)
        .output()
        .unwrap();
    assert!(
        stock.status.success(),
        "{}",
        String::from_utf8_lossy(&stock.stderr)
    );
    let stock = Command::new(stock_image).output().unwrap();
    assert!(!stock.status.success());
    for workers in [1, 8] {
        let image = dir.join(format!("native-{workers}"));
        let build = Command::new(env!("CARGO_BIN_EXE_jr-rust"))
            .arg(&src)
            .arg("--workers")
            .arg(workers.to_string())
            .arg("-o")
            .arg(&image)
            .output()
            .unwrap();
        assert!(
            build.status.success(),
            "{}",
            String::from_utf8_lossy(&build.stderr)
        );
        let native = Command::new(image).output().unwrap();
        assert!(!native.status.success());
        assert_eq!(native.stdout, stock.stdout);
        assert_eq!(native.stdout, b"before\n");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn shared_slice_get_matches_stock() {
    #[path = "support/closure_oracle.rs"]
    mod closure_oracle;
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "slice-get",
        &[
            Case {
                name: "checked_indices_and_empty",
                source: "fn at<T>(s:&[T],i:usize)->Option<&T>{s.get(i)}fn main(){let a=[10i32,20,30];println!(\"{:?} {:?} {:?}\",at(&a,0usize),at(&a,2usize),at(&a,3usize));let empty:[i32;0]=[];println!(\"{:?} {:?}\",at(&empty,0usize),at(&a,usize::MAX));}",
                expect: Expect::Run("Some(10) Some(30) None\nNone None\n"),
            },
            Case {
                name: "retains_owner_loan",
                source: "fn main(){let mut a=[1i32,2i32];let r=a.get(0usize);a[0usize]=3;println!(\"{:?}\",r);}",
                expect: Expect::Invalid("E0506"),
            },
            Case {
                name: "noncopy_and_zero_sized_elements",
                source: "struct D{value:i32}fn at<T>(s:&[T],i:usize)->Option<&T>{s.get(i)}fn main(){let a=[D{value:17}];match at(&a,0usize){Some(d)=>println!(\"{}\",d.value),None=>println!(\"missing\")};let z=[();2];println!(\"{} {}\",at(&z,1usize).is_some(),at(&z,2usize).is_none());}",
                expect: Expect::Run("17\ntrue true\n"),
            },
            Case {
                name: "no_escape",
                source: "fn main(){let r={let a=[1i32,2i32];a.get(0usize)};println!(\"{:?}\",r);}",
                expect: Expect::Invalid("E0597"),
            },
        ],
    );
}

/// `[T]::iter` over slice-backed `slice::Iter`, array receivers reaching slice
/// methods through stock's final unsizing probe step, and `Range<int>` as an
/// `Iterator` (corpus 06/11/15 reductions).
#[test]
fn slice_iter_array_unsize_and_range_iterator_match_stock() {
    #[path = "support/closure_oracle.rs"]
    mod closure_oracle;
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "slice-iter-unsize",
        &[
            Case {
                name: "slice_and_vec_iter",
                source: r#"
fn total(s: &[i64]) -> i64 { let mut t = 0; for x in s.iter() { t += *x; } t }
fn main() {
    let v = vec![1i64, 2, 3];
    let s: &[i64] = &v[1..];
    for x in s.iter() { println!("{}", x); }
    println!("{}", total(v.as_slice()));
    let e: &[i64] = &v[3..];
    let mut it = e.iter();
    println!("{}", it.next().is_none());
}
"#,
                expect: Expect::Run("2\n3\n6\ntrue\n"),
            },
            Case {
                name: "array_receivers_unsize",
                source: r#"
fn main() {
    let a = [4i64, 5, 6];
    let r = &a;
    let n = a.iter().map(|x| *x * 2).count();
    let f = r.iter().fold(0i64, |acc, x| acc + *x);
    println!("{} {} {} {:?}", n, f, a.is_empty(), a.first());
    let m = [[1i64, 2], [3, 4]];
    for row in m.iter() { println!("{}", row.iter().fold(0i64, |s, x| s + *x)); }
}
"#,
                expect: Expect::Run("3 15 false Some(4)\n3\n7\n"),
            },
            Case {
                name: "range_iterator",
                source: r#"
fn main() {
    let a = [10i64, 20, 30];
    let t = (0..3usize).map(|i| a[i]).fold(0i64, |s: i64, x: i64| s + x);
    let mut r = 5..7i64;
    let x = r.next();
    let y = r.next();
    let z = r.next();
    println!("{} {:?} {:?} {:?}", t, x, y, z);
    println!("{}", (3u8..3).count());
}
"#,
                expect: Expect::Run("60 Some(5) Some(6) None\n0\n"),
            },
            Case {
                name: "empty_and_zero_sized_iteration",
                source: "fn main(){let empty:[i32;0]=[];let units=[();3];println!(\"{} {}\",empty.iter().count(),units.iter().count());}",
                expect: Expect::Run("0 3\n"),
            },
            Case {
                name: "iterator_keeps_owner_borrowed",
                source: "fn main(){let mut v=vec![1i32,2];let mut it=v.iter();v.push(3);println!(\"{:?}\",it.next());}",
                expect: Expect::Invalid("E0502"),
            },
            Case {
                name: "iterator_cannot_escape_owner",
                source: "fn main(){let mut it={let v=vec![1i32,2];v.iter()};println!(\"{:?}\",it.next());}",
                expect: Expect::Invalid("E0597"),
            },
            Case {
                name: "array_iter_item_mismatch_unused",
                source: "fn bad() { let a = [1i64, 2]; let _: i64 = a.iter().count(); }\nfn main() { println!(\"x\"); }\n",
                expect: Expect::Invalid("E0308"),
            },
            Case {
                name: "range_item_mismatch_unused",
                source: "fn bad() { let mut r = 0..2u8; let _: Option<i64> = r.next(); }\nfn main() { println!(\"x\"); }\n",
                expect: Expect::Invalid("E0308"),
            },
        ],
    );
}

#[test]
#[ignore = "pending general heap-stored reference lifetime authority; not supported by chunks milestone"]
fn collected_slice_references_require_heap_lifetime_authority() {
    #[path = "support/closure_oracle.rs"]
    mod closure_oracle;
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "heap-slice-retention",
        &[
            Case {
                name: "collected_slices",
                source: r#"fn main(){let a=[1i32,2,3,4];let saved:Vec<&[i32]>=a.chunks(3).collect();println!("{} {}",saved[0][2],saved[1][0]);}"#,
                expect: Expect::Run("3 4\n"),
            },
            Case {
                name: "collected_chunks_keep_owner_borrowed",
                source: r#"fn main(){let mut v=vec![1i32,2];let saved:Vec<&[i32]>=v.chunks(1).collect();v.push(3);println!("{}",saved[0][0]);}"#,
                expect: Expect::Invalid("E0502"),
            },
            Case {
                name: "collected_chunk_outlives_owner",
                source: r#"fn main(){let c;{let v=vec![1i32,2];let saved:Vec<&[i32]>=v.chunks(1).collect();c=saved[0];}println!("{}",c[0]);}"#,
                expect: Expect::Invalid("E0597"),
            },
        ],
    );
}

#[test]
fn slice_chunks_match_stock() {
    #[path = "support/closure_oracle.rs"]
    mod closure_oracle;
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "slice-chunks",
        &[
            Case {
                name: "chunk_sums",
                source: r#"fn main(){let v=vec![1i32,2,3,4,5,6,7];let sums:Vec<i32>=v.chunks(3).map(|c|c.iter().sum::<i32>()).collect();println!("{:?}",sums);let a=[1i32,2,3,4,5];let mut c=a.chunks(2);println!("{:?} {:?} {:?} {}",c.next_back().unwrap(),c.next().unwrap(),c.next_back().unwrap(),c.next().is_none());}"#,
                expect: Expect::Run(
                    "[6, 15, 7]
[5] [1, 2] [3, 4] true
",
                ),
            },
            Case {
                name: "empty_huge_and_zst",
                source: r#"fn main(){let a:[i32;0]=[];let b=[1i32,2];let z=[();5];println!("{} {} {}",a.chunks(usize::MAX).count(),b.chunks(usize::MAX).next().unwrap().len(),z.chunks(2).rev().map(|c|c.len()).sum::<usize>());}"#,
                expect: Expect::Run(
                    "0 2 5
",
                ),
            },
            Case {
                name: "nested_shared_descriptors",
                source: r#"fn split(s:&[i32])->(Option<&[i32]>,&[i32]){(Some(&s[..2]),&s[2..])}fn main(){let a=[1i32,2,3,4];let (x,y)=split(&a);println!("{:?} {:?}",x.unwrap(),y);}"#,
                expect: Expect::Run("[1, 2] [3, 4]\n"),
            },
            Case {
                name: "chunks_keep_owner_borrowed",
                source: r#"fn main(){let mut v=vec![1i32,2];let mut c=v.chunks(1);v.push(3);println!("{:?}",c.next());}"#,
                expect: Expect::Invalid("E0502"),
            },
            Case {
                name: "chunk_outlives_owner",
                source: r#"fn main(){let c;{let v=vec![1i32,2];c=v.chunks(1).next().unwrap();}println!("{:?}",c);}"#,
                expect: Expect::Invalid("E0597"),
            },
            Case {
                name: "chunks_exact_remainder_and_nested_loops",
                source: r#"fn sum(bytes:&[u8])->(u32,usize,usize){let (bytes,rest)=bytes.split_at(bytes.len()-bytes.len()%4);let it=bytes.chunks_exact(8);let tail=it.remainder();let mut t=0u32;let mut n=0usize;for chunk in it{for word in chunk.chunks_exact(4){t+=u32::from(word[0])+u32::from(word[3]);}n+=1;}for word in tail.chunks_exact(4){t+=u32::from(word[1]);}for &b in rest.iter(){t+=u32::from(b);}(t,n,tail.len())}fn main(){let d:[u8;23]=[1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23];let (t,c,r)=sum(&d);let mut back=d.chunks_exact(5);let last=back.next_back().unwrap();let mut nested=0usize;for a in d.chunks(8){for x in a.iter(){nested+=*x as usize;}}println!("{} {} {} {} {} {} {}",t,c,r,last[0],back.remainder().len(),back.count(),nested);}"#,
                expect: Expect::Run("152 2 4 16 3 3 276\n"),
            },
            Case {
                name: "loop_call_result_kept_across_iterations",
                source: r#"fn main(){let mut w=vec![1i32];let mut n=0usize;for _ in 0..2{let s=w.chunks(1).next().unwrap();n+=s.len();w.push(2);n+=s.len();}println!("{}",n);}"#,
                expect: Expect::Invalid("E0502"),
            },
        ],
    );
}

#[test]
fn zero_chunk_panics_like_stock() {
    let source =
        "fn main(){let a=[1i32,2];println!(\"before\");let _=a.chunks(0);println!(\"after\");}";
    let dir = scratch("zero-chunk");
    let src = dir.join("main.rs");
    std::fs::write(&src, source).unwrap();
    let stock = dir.join("stock");
    let built = Command::new(stock_rustc::stock_rustc())
        .env_remove("RUSTC_BOOTSTRAP")
        .args(["--edition=2024", "-Cpanic=abort"])
        .arg(&src)
        .arg("-o")
        .arg(&stock)
        .output()
        .unwrap();
    assert!(built.status.success(), "{built:?}");
    let expected = Command::new(stock).output().unwrap();
    assert!(!expected.status.success());
    assert_eq!(expected.stdout, b"before\n");
    for workers in ["1", "8"] {
        let check = Command::new(env!("CARGO_BIN_EXE_jr-rust"))
            .arg(&src)
            .args(["--check", "--workers", workers])
            .output()
            .unwrap();
        assert!(check.status.success(), "{check:?}");
        let image = dir.join(format!("native-{workers}"));
        let built = Command::new(env!("CARGO_BIN_EXE_jr-rust"))
            .arg(&src)
            .arg("-o")
            .arg(&image)
            .args(["--workers", workers])
            .output()
            .unwrap();
        assert!(built.status.success(), "{built:?}");
        let actual = Command::new(image).output().unwrap();
        assert!(!actual.status.success());
        assert_eq!(actual.stdout, expected.stdout);
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn double_ended_slice_iteration_matches_stock() {
    #[path = "support/closure_oracle.rs"]
    mod closure_oracle;
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "slice-double-ended",
        &[
            Case {
                name: "mixed_front_back_and_reverse",
                source: r#"fn main(){let a=[1i32,2,3,4];let mut it=a.iter();println!("{} {} {} {} {}",it.next_back().unwrap(),it.next().unwrap(),it.next_back().unwrap(),it.next().unwrap(),it.next_back().is_none());let v:Vec<i32>=a.iter().rev().copied().collect();println!("{} {}",v[0],v[3]);}"#,
                expect: Expect::Run("4 1 3 2 true\n4 1\n"),
            },
            Case {
                name: "empty_singleton_and_zero_sized",
                source: r#"fn main(){let empty:[i32;0]=[];let mut e=empty.iter();let one=[7i32];let mut o=one.iter();let units=[();3];let mut z=units.iter();println!("{} {} {} {} {} {} {}",e.next_back().is_none(),o.next_back().unwrap(),o.next().is_none(),z.next_back().is_some(),z.next().is_some(),z.next_back().is_some(),z.next().is_none());}"#,
                expect: Expect::Run("true 7 true true true true true\n"),
            },
            Case {
                name: "reverse_keeps_owner_borrowed",
                source: r#"fn main(){let mut v=vec![1i32,2];let mut it=v.iter().rev();v.push(3);println!("{:?}",it.next());}"#,
                expect: Expect::Invalid("E0502"),
            },
            Case {
                name: "reverse_cannot_escape_owner",
                source: r#"fn main(){let mut it={let v=vec![1i32,2];v.iter().rev()};println!("{:?}",it.next());}"#,
                expect: Expect::Invalid("E0597"),
            },
        ],
    );
}

#[test]
fn array_equality_matches_stock() {
    check(
        "array-equality",
        &[
            Case {
                name: "values_literals_fields_and_temporaries",
                source: r#"fn mk(n: i32) -> [i32; 3] { println!("mk {n}"); [n, 2, 3] }
struct S { a: [u8; 2] }
fn main() {
    let a = [1, 2, 3]; let b = [1, 2, 3]; let c = [1, 2, 4];
    println!("{} {} {} {}", a == b, a != b, a == c, a != c);
    println!("{} {}", a == [1, 2, 3], [1, 2, 4] == a);
    println!("{} {}", mk(1) == mk(1), mk(1) != mk(2));
    let s = [String::from("x"), String::from("y")]; let t = [String::from("x"), String::from("z")];
    println!("{} {}", s == t, s != t); println!("{}", s[0]);
    let p = S { a: [1, 2] }; let q = S { a: [1, 2] };
    println!("{} {}", p.a == q.a, p.a == [1u8, 3]);
    let x: [u8; 4] = [7; 4]; println!("{} {}", x == [7; 4], x == [7u8, 7, 7, 8]);
    let f = [1.0, f64::NAN]; println!("{} {}", f == f, f != f);
    println!("{}", [1.0, f64::NAN] == [1.0, f64::NAN]);
    let e: [i32; 0] = []; println!("{}", e == e);
    if a == b && a != c { println!("ok"); }
}"#,
                expect: Expect::Run(
                    "true false false true\ntrue false\nmk 1\nmk 1\nmk 1\nmk 2\ntrue true\nfalse true\nx\ntrue false\ntrue false\nfalse true\nfalse\ntrue\nok\n",
                ),
            },
            // Stock calls the element `ne` for both `==` and `!=` on arrays,
            // and `eq` / `ne` respectively on tuples, in order, stopping early.
            Case {
                name: "user_impl_call_order_and_count",
                source: r#"struct P(i32);
impl PartialEq for P {
    fn eq(&self, o: &P) -> bool { println!("eq {} {}", self.0, o.0); self.0 == o.0 }
    fn ne(&self, o: &P) -> bool { println!("ne {} {}", self.0, o.0); self.0 != o.0 }
}
fn mk(n: i32) -> [P; 3] { println!("mk {n}"); [P(n), P(2), P(3)] }
fn main() {
    let a = [P(1), P(2), P(3)]; let b = [P(1), P(9), P(3)];
    println!("{}", a == b); println!("{}", a != b); println!("{}", a == a); println!("{}", a != a);
    println!("{}", mk(1) == mk(1)); println!("{}", mk(1) != mk(5));
    let t = (P(1), P(2)); let u = (P(1), P(7));
    println!("{}", t == u); println!("{}", t != u); println!("{}", t == t); println!("{}", t != t);
    let e: [P; 0] = []; println!("{} {}", e == e, e != e);
}"#,
                expect: Expect::Run(
                    "ne 1 1\nne 2 9\nfalse\nne 1 1\nne 2 9\ntrue\nne 1 1\nne 2 2\nne 3 3\ntrue\nne 1 1\nne 2 2\nne 3 3\nfalse\nmk 1\nmk 1\nne 1 1\nne 2 2\nne 3 3\ntrue\nmk 1\nmk 5\nne 1 5\ntrue\neq 1 1\neq 2 7\nfalse\nne 1 1\nne 2 7\ntrue\neq 1 1\neq 2 2\ntrue\nne 1 1\nne 2 2\nfalse\ntrue false\n",
                ),
            },
            Case {
                name: "different_lengths",
                source: r#"fn main() { let a = [1, 2, 3]; let b = [1, 2]; println!("{}", a == b); }"#,
                expect: Expect::Invalid("E0277"),
            },
            Case {
                name: "element_without_partial_eq",
                source: r#"struct N(i32);
fn main() { let a = [N(1)]; let b = [N(1)]; println!("{}", a == b); }"#,
                expect: Expect::Invalid("E0369"),
            },
            Case {
                name: "different_element_types",
                source: r#"fn main() { let a = [1i32, 2]; let b = [1i64, 2]; println!("{}", a == b); }"#,
                expect: Expect::Invalid("E0277"),
            },
        ],
    );
}

#[test]
fn inclusive_and_reverse_integer_ranges_match_stock() {
    #[path = "support/closure_oracle.rs"]
    mod closure_oracle;
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "inclusive-integer-ranges",
        &[
            Case {
                name: "inclusive_i8",
                source: "fn main(){ let mut top=(i8::MAX-1i8)..=i8::MAX; let a=top.next().unwrap(); let b=top.next_back().unwrap(); println!(\"{} {} {}\",a==i8::MAX-1i8,b==i8::MAX,top.next().is_none()); let mut low=(i8::MIN..=i8::MIN+1i8).rev(); println!(\"{} {} {}\",low.next().unwrap()==i8::MIN+1i8,low.next().unwrap()==i8::MIN,low.next().is_none()); let v:Vec<i8>=(1i8..=3i8).collect(); let w:Vec<i8>=(1i8..4i8).rev().collect(); println!(\"{} {} {} {}\",v.len(),v[2],w[0],(3i8..=2i8).count()); }",
                expect: Expect::Run("true true true\ntrue true true\n3 3 3 0\n"),
            },
            Case {
                name: "inclusive_i16",
                source: "fn main(){ let mut top=(i16::MAX-1i16)..=i16::MAX; let a=top.next().unwrap(); let b=top.next_back().unwrap(); println!(\"{} {} {}\",a==i16::MAX-1i16,b==i16::MAX,top.next().is_none()); let mut low=(i16::MIN..=i16::MIN+1i16).rev(); println!(\"{} {} {}\",low.next().unwrap()==i16::MIN+1i16,low.next().unwrap()==i16::MIN,low.next().is_none()); let v:Vec<i16>=(1i16..=3i16).collect(); let w:Vec<i16>=(1i16..4i16).rev().collect(); println!(\"{} {} {} {}\",v.len(),v[2],w[0],(3i16..=2i16).count()); }",
                expect: Expect::Run("true true true\ntrue true true\n3 3 3 0\n"),
            },
            Case {
                name: "inclusive_i32",
                source: "fn main(){ let mut top=(i32::MAX-1i32)..=i32::MAX; let a=top.next().unwrap(); let b=top.next_back().unwrap(); println!(\"{} {} {}\",a==i32::MAX-1i32,b==i32::MAX,top.next().is_none()); let mut low=(i32::MIN..=i32::MIN+1i32).rev(); println!(\"{} {} {}\",low.next().unwrap()==i32::MIN+1i32,low.next().unwrap()==i32::MIN,low.next().is_none()); let v:Vec<i32>=(1i32..=3i32).collect(); let w:Vec<i32>=(1i32..4i32).rev().collect(); println!(\"{} {} {} {}\",v.len(),v[2],w[0],(3i32..=2i32).count()); }",
                expect: Expect::Run("true true true\ntrue true true\n3 3 3 0\n"),
            },
            Case {
                name: "inclusive_i64",
                source: "fn main(){ let mut top=(i64::MAX-1i64)..=i64::MAX; let a=top.next().unwrap(); let b=top.next_back().unwrap(); println!(\"{} {} {}\",a==i64::MAX-1i64,b==i64::MAX,top.next().is_none()); let mut low=(i64::MIN..=i64::MIN+1i64).rev(); println!(\"{} {} {}\",low.next().unwrap()==i64::MIN+1i64,low.next().unwrap()==i64::MIN,low.next().is_none()); let v:Vec<i64>=(1i64..=3i64).collect(); let w:Vec<i64>=(1i64..4i64).rev().collect(); println!(\"{} {} {} {}\",v.len(),v[2],w[0],(3i64..=2i64).count()); }",
                expect: Expect::Run("true true true\ntrue true true\n3 3 3 0\n"),
            },
            Case {
                name: "inclusive_i128",
                source: "fn main(){ let mut top=(i128::MAX-1i128)..=i128::MAX; let a=top.next().unwrap(); let b=top.next_back().unwrap(); println!(\"{} {} {}\",a==i128::MAX-1i128,b==i128::MAX,top.next().is_none()); let mut low=(i128::MIN..=i128::MIN+1i128).rev(); println!(\"{} {} {}\",low.next().unwrap()==i128::MIN+1i128,low.next().unwrap()==i128::MIN,low.next().is_none()); let v:Vec<i128>=(1i128..=3i128).collect(); let w:Vec<i128>=(1i128..4i128).rev().collect(); println!(\"{} {} {} {}\",v.len(),v[2],w[0],(3i128..=2i128).count()); }",
                expect: Expect::Run("true true true\ntrue true true\n3 3 3 0\n"),
            },
            Case {
                name: "inclusive_isize",
                source: "fn main(){ let mut top=(isize::MAX-1isize)..=isize::MAX; let a=top.next().unwrap(); let b=top.next_back().unwrap(); println!(\"{} {} {}\",a==isize::MAX-1isize,b==isize::MAX,top.next().is_none()); let mut low=(isize::MIN..=isize::MIN+1isize).rev(); println!(\"{} {} {}\",low.next().unwrap()==isize::MIN+1isize,low.next().unwrap()==isize::MIN,low.next().is_none()); let v:Vec<isize>=(1isize..=3isize).collect(); let w:Vec<isize>=(1isize..4isize).rev().collect(); println!(\"{} {} {} {}\",v.len(),v[2],w[0],(3isize..=2isize).count()); }",
                expect: Expect::Run("true true true\ntrue true true\n3 3 3 0\n"),
            },
            Case {
                name: "inclusive_u8",
                source: "fn main(){ let mut top=(u8::MAX-1u8)..=u8::MAX; let a=top.next().unwrap(); let b=top.next_back().unwrap(); println!(\"{} {} {}\",a==u8::MAX-1u8,b==u8::MAX,top.next().is_none()); let mut low=(u8::MIN..=u8::MIN+1u8).rev(); println!(\"{} {} {}\",low.next().unwrap()==u8::MIN+1u8,low.next().unwrap()==u8::MIN,low.next().is_none()); let v:Vec<u8>=(1u8..=3u8).collect(); let w:Vec<u8>=(1u8..4u8).rev().collect(); println!(\"{} {} {} {}\",v.len(),v[2],w[0],(3u8..=2u8).count()); }",
                expect: Expect::Run("true true true\ntrue true true\n3 3 3 0\n"),
            },
            Case {
                name: "inclusive_u16",
                source: "fn main(){ let mut top=(u16::MAX-1u16)..=u16::MAX; let a=top.next().unwrap(); let b=top.next_back().unwrap(); println!(\"{} {} {}\",a==u16::MAX-1u16,b==u16::MAX,top.next().is_none()); let mut low=(u16::MIN..=u16::MIN+1u16).rev(); println!(\"{} {} {}\",low.next().unwrap()==u16::MIN+1u16,low.next().unwrap()==u16::MIN,low.next().is_none()); let v:Vec<u16>=(1u16..=3u16).collect(); let w:Vec<u16>=(1u16..4u16).rev().collect(); println!(\"{} {} {} {}\",v.len(),v[2],w[0],(3u16..=2u16).count()); }",
                expect: Expect::Run("true true true\ntrue true true\n3 3 3 0\n"),
            },
            Case {
                name: "inclusive_u32",
                source: "fn main(){ let mut top=(u32::MAX-1u32)..=u32::MAX; let a=top.next().unwrap(); let b=top.next_back().unwrap(); println!(\"{} {} {}\",a==u32::MAX-1u32,b==u32::MAX,top.next().is_none()); let mut low=(u32::MIN..=u32::MIN+1u32).rev(); println!(\"{} {} {}\",low.next().unwrap()==u32::MIN+1u32,low.next().unwrap()==u32::MIN,low.next().is_none()); let v:Vec<u32>=(1u32..=3u32).collect(); let w:Vec<u32>=(1u32..4u32).rev().collect(); println!(\"{} {} {} {}\",v.len(),v[2],w[0],(3u32..=2u32).count()); }",
                expect: Expect::Run("true true true\ntrue true true\n3 3 3 0\n"),
            },
            Case {
                name: "inclusive_u64",
                source: "fn main(){ let mut top=(u64::MAX-1u64)..=u64::MAX; let a=top.next().unwrap(); let b=top.next_back().unwrap(); println!(\"{} {} {}\",a==u64::MAX-1u64,b==u64::MAX,top.next().is_none()); let mut low=(u64::MIN..=u64::MIN+1u64).rev(); println!(\"{} {} {}\",low.next().unwrap()==u64::MIN+1u64,low.next().unwrap()==u64::MIN,low.next().is_none()); let v:Vec<u64>=(1u64..=3u64).collect(); let w:Vec<u64>=(1u64..4u64).rev().collect(); println!(\"{} {} {} {}\",v.len(),v[2],w[0],(3u64..=2u64).count()); }",
                expect: Expect::Run("true true true\ntrue true true\n3 3 3 0\n"),
            },
            Case {
                name: "inclusive_u128",
                source: "fn main(){ let mut top=(u128::MAX-1u128)..=u128::MAX; let a=top.next().unwrap(); let b=top.next_back().unwrap(); println!(\"{} {} {}\",a==u128::MAX-1u128,b==u128::MAX,top.next().is_none()); let mut low=(u128::MIN..=u128::MIN+1u128).rev(); println!(\"{} {} {}\",low.next().unwrap()==u128::MIN+1u128,low.next().unwrap()==u128::MIN,low.next().is_none()); let v:Vec<u128>=(1u128..=3u128).collect(); let w:Vec<u128>=(1u128..4u128).rev().collect(); println!(\"{} {} {} {}\",v.len(),v[2],w[0],(3u128..=2u128).count()); }",
                expect: Expect::Run("true true true\ntrue true true\n3 3 3 0\n"),
            },
            Case {
                name: "inclusive_usize",
                source: "fn main(){ let mut top=(usize::MAX-1usize)..=usize::MAX; let a=top.next().unwrap(); let b=top.next_back().unwrap(); println!(\"{} {} {}\",a==usize::MAX-1usize,b==usize::MAX,top.next().is_none()); let mut low=(usize::MIN..=usize::MIN+1usize).rev(); println!(\"{} {} {}\",low.next().unwrap()==usize::MIN+1usize,low.next().unwrap()==usize::MIN,low.next().is_none()); let v:Vec<usize>=(1usize..=3usize).collect(); let w:Vec<usize>=(1usize..4usize).rev().collect(); println!(\"{} {} {} {}\",v.len(),v[2],w[0],(3usize..=2usize).count()); }",
                expect: Expect::Run("true true true\ntrue true true\n3 3 3 0\n"),
            },
            Case {
                name: "wrong_inclusive_item_unused",
                source: "fn bad(){let mut r=1u8..=2;let _:Option<i32>=r.next();}fn main(){}",
                expect: Expect::Invalid("E0308"),
            },
        ],
    );
}

#[test]
fn promoted_large_byte_repeats_match_stock() {
    #[path = "support/closure_oracle.rs"]
    mod closure_oracle;
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "large-byte-repeats",
        &[
            Case {
                name: "promoted_shared_u8_repeats",
                source: "fn s(x:&[u8])->u64{let mut t=0u64; for b in x {t+=*b as u64;} t} fn main(){ println!(\"{} {} {}\", s(&[0xff; 1<<20]), s(&[7u8; 70000]), s(&[0; 65536])); let r: &[u8; 100000] = &[3; 100000]; println!(\"{} {}\", r.len(), s(r)); assert_eq!(s(&[1; 1024 * 1024 - 1024]), 1047552); }",
                expect: Expect::Run("267386880 490000 0\n100000 300000\n"),
            },
            Case {
                name: "out_of_range_byte_literal",
                source: "fn s(x:&[u8])->usize{x.len()} fn main(){ s(&[256; 70000]); }",
                expect: Expect::Invalid("out of range"),
            },
            Case {
                name: "element_type_mismatch",
                source: "fn s(x:&[u16])->usize{x.len()} fn main(){ s(&[0u8; 70000]); }",
                expect: Expect::Invalid("E0308"),
            },
        ],
    );
}

#[test]
fn unpromoted_large_arrays_remain_explicitly_unsupported() {
    for (name, source) in [
        (
            "nonliteral",
            "fn s(x:&[u8])->usize{x.len()} fn main(){ let b = 1u8; println!(\"{}\", s(&[b; 70000])); }",
        ),
        (
            "mutable",
            "fn s(x:&mut [u8])->usize{x.len()} fn main(){ println!(\"{}\", s(&mut [0u8; 70000])); }",
        ),
        (
            "wide_element",
            "fn s(x:&[u32])->usize{x.len()} fn main(){ println!(\"{}\", s(&[0u32; 70000])); }",
        ),
        (
            "owned_local",
            "fn main(){ let a = [0u8; 70000]; println!(\"{}\", a.len()); }",
        ),
    ] {
        let root = scratch(&format!("large-array-{name}"));
        let path = root.join("input.rs");
        std::fs::write(&path, source).unwrap();
        let stock = Command::new(stock_rustc::stock_rustc())
            .args(["--edition=2024", "-Cpanic=abort"])
            .arg(&path)
            .arg("-o")
            .arg(root.join("stock"))
            .output()
            .unwrap();
        assert!(stock.status.success(), "{name}: {stock:?}");
        for workers in [1, 8] {
            let error = justrust_engine::compiler::compile(source.into(), workers, false)
                .err()
                .unwrap_or_else(|| panic!("{name}: large array must not build"))
                .to_string();
            assert!(error.contains("unsupported"), "{name}: {error}");
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn slice_and_vec_iter_mut_match_stock() {
    #[path = "support/closure_oracle.rs"]
    mod closure_oracle;
    use closure_oracle::{Case, Expect};
    let invalid = |name, source, code| Case {
        name,
        source,
        expect: Expect::Invalid(code),
    };
    closure_oracle::check(
        "slice-vec-iter-mut",
        &[
            Case {
                name: "vec_and_slice_parameter_loops_two_live_items",
                source: r#"fn bump(s: &mut [i32]) { for x in s.iter_mut() { *x += 1; } }
fn scale(s: &mut [i32]) { for x in s { *x *= 2; } }
fn main() {
    let mut v: Vec<i32> = vec![1, 2, 3, 4];
    for x in v.iter_mut() { *x *= 10; }
    for x in &mut v { *x += 1; }
    bump(&mut v);
    scale(&mut v);
    println!("{:?}", v);
    let mut it = v.iter_mut();
    let a = it.next().unwrap();
    let b = it.next().unwrap();
    *a += 100;
    *b += 200;
    println!("{:?}", v);
}"#,
                expect: Expect::Run("[24, 44, 64, 84]\n[124, 244, 64, 84]\n"),
            },
            Case {
                name: "struct_elements_and_if_let_next",
                source: r#"struct Counter { n: u32 }
fn main() {
    let mut cs = vec![Counter { n: 0 }, Counter { n: 5 }];
    for c in cs.iter_mut() { c.n += 2; }
    if let Some(first) = cs.iter_mut().next() { first.n = 100; }
    println!("{} {}", cs[0].n, cs[1].n);
}"#,
                expect: Expect::Run("100 7\n"),
            },
            Case {
                name: "boxed_dyn_trait_elements",
                source: r#"trait S { fn go(&mut self) -> u32; }
struct A { n: u32 }
impl S for A { fn go(&mut self) -> u32 { self.n += 1; self.n } }
fn run(s: &mut dyn S) -> u32 { s.go() }
fn main() {
    let mut v: Vec<Box<dyn S>> = vec![Box::new(A { n: 1 })];
    for s in v.iter_mut() { println!("{}", run(s.as_mut())); }
}"#,
                expect: Expect::Run("2\n"),
            },
            Case {
                name: "string_elements_and_for_each",
                source: r#"fn main() {
    let mut names = vec![String::from("a"), String::from("b")];
    for s in names.iter_mut() { s.push('!'); }
    names.iter_mut().for_each(|s| s.push('?'));
    println!("{:?}", names);
}"#,
                expect: Expect::Run("[\"a!?\", \"b!?\"]\n"),
            },
            Case {
                name: "array_reference_enumerate_and_rev",
                source: r#"fn main() {
    let mut a: [u8; 3] = [5, 6, 7];
    for x in &mut a { *x += 1; }
    let s: &mut [u8] = &mut a;
    for (i, x) in s.iter_mut().enumerate() { *x += i as u8; }
    for x in s.iter_mut().rev() { *x += 1; }
    println!("{:?}", a);
}"#,
                expect: Expect::Run("[7, 9, 11]\n"),
            },
            Case {
                name: "direct_array_receiver_iter_mut",
                source: r#"fn main() {
    let mut a: [u8; 3] = [5, 6, 7];
    for x in a.iter_mut() { *x += 1; }
    println!("{:?}", a);
}"#,
                expect: Expect::Run("[6, 7, 8]\n"),
            },
            Case {
                name: "map_sum_and_nested_rows",
                source: r#"fn main() {
    let mut v: Vec<i32> = vec![1, 2, 3];
    let total: i32 = v.iter_mut().map(|x| { *x += 1; *x }).sum();
    println!("{} {:?}", total, v);
    let mut g: Vec<Vec<usize>> = vec![vec![0, 0, 0], vec![0, 0, 0]];
    for (r, row) in g.iter_mut().enumerate() {
        for (c, cell) in row.iter_mut().enumerate() { *cell = r * 3 + c; }
    }
    println!("{:?}", g);
}"#,
                expect: Expect::Run("9 [2, 3, 4]\n[[0, 1, 2], [3, 4, 5]]\n"),
            },
            invalid(
                "second_iter_mut_while_item_live",
                "fn main(){let mut v:Vec<i32>=vec![1,2];let a=v.iter_mut().next().unwrap();let b=v.iter_mut().next().unwrap();*a+=1;*b+=1;}",
                "E0499",
            ),
            invalid(
                "item_kept_across_push",
                "fn main(){let mut v:Vec<i32>=vec![1,2];let mut it=v.iter_mut();let a=it.next().unwrap();v.push(3);*a+=1;}",
                "E0499",
            ),
            invalid(
                "two_iterators_over_one_vec",
                "fn main(){let mut v:Vec<i32>=vec![1,2];let mut i1=v.iter_mut();let mut i2=v.iter_mut();let a=i1.next().unwrap();let b=i2.next().unwrap();*a+=1;*b+=1;}",
                "E0499",
            ),
            // Stock reports E0515. We reject the same return as E0499.
            invalid(
                "item_returned_from_local_vec",
                "fn first()->&'static mut i32{let mut v:Vec<i32>=vec![1,2];let mut it=v.iter_mut();it.next().unwrap()} fn main(){*first()+=1;}",
                "E0515",
            ),
            invalid(
                "owner_read_while_item_live",
                "fn main(){let mut v:Vec<i32>=vec![1,2];let mut it=v.iter_mut();let a=it.next().unwrap();let n=v.len();*a+=n as i32;}",
                "E0502",
            ),
            invalid(
                "shared_borrow_while_item_live",
                "fn main(){let mut v:Vec<i32>=vec![1,2];let mut it=v.iter_mut();let a=it.next().unwrap();let s=&v;*a+=1;println!(\"{:?}\",s);}",
                "E0502",
            ),
            invalid(
                "owner_moved_while_item_live",
                "fn main(){let mut v:Vec<i32>=vec![1,2];let mut it=v.iter_mut();let a=it.next().unwrap();drop(v);*a+=1;}",
                "E0505",
            ),
            // Stock reports E0597. We reject the same escape as E0499.
            invalid(
                "item_outlives_owner",
                "fn main(){let r;{let mut v:Vec<i32>=vec![1,2];let mut it=v.iter_mut();r=it.next().unwrap();}*r+=1;}",
                "E0597",
            ),
            invalid(
                "nested_iter_mut_inside_loop",
                "fn main(){let mut v:Vec<i32>=vec![1,2];for x in v.iter_mut(){let y=v.iter_mut().next().unwrap();*x+=*y;}}",
                "E0499",
            ),
            // Stock reports E0503. We reject the same write as E0502.
            invalid(
                "slice_write_while_item_live",
                "fn main(){let mut v:Vec<i32>=vec![1,2];let s:&mut[i32]=&mut v;let mut it=s.iter_mut();let a=it.next().unwrap();s[0]=9;*a+=1;}",
                "E0503",
            ),
            invalid(
                "immutable_owner",
                "fn main(){let v:Vec<i32>=vec![1,2];for x in v.iter_mut(){*x+=1;}}",
                "E0596",
            ),
            // The fragment is matched against stock only. Our own rejection of
            // the next case comes from move analysis without a printed code.
            invalid(
                "item_moved_then_used",
                "fn main(){let mut v:Vec<i32>=vec![1,2];let mut it=v.iter_mut();let a:&mut i32=it.next().unwrap();let b=a;*a=1;*b=2;}",
                "E0382",
            ),
            invalid(
                "push_inside_iter_mut_loop",
                "fn main(){let mut v:Vec<i32>=vec![1,2];for x in v.iter_mut(){v.push(*x);}}",
                "E0499",
            ),
            // A user iterator must not hand out the same element twice, and its
            // `next` body is rejected even when the iterator is never used.
            invalid(
                "unused_generic_iterator_repeats_its_item",
                "struct Twice<'a,T>{item:&'a mut T} impl<'a,T> Iterator for Twice<'a,T>{type Item=&'a mut T;fn next(&mut self)->Option<&'a mut T>{Some(&mut *self.item)}} fn main(){}",
                "lifetime may not live long enough",
            ),
            invalid(
                "concrete_iterator_repeats_its_item",
                "struct Twice<'a>{item:&'a mut i32} impl<'a> Iterator for Twice<'a>{type Item=&'a mut i32;fn next(&mut self)->Option<&'a mut i32>{Some(&mut *self.item)}} fn main(){let mut x:i32=1;let mut t=Twice{item:&mut x};let a:&mut i32=t.next().unwrap();let b:&mut i32=t.next().unwrap();*a+=1;*b+=1;}",
                "lifetime may not live long enough",
            ),
            invalid(
                "unused_iterator_restarts_vec_iter_mut",
                "struct Same<'a,T>{items:&'a mut Vec<T>} impl<'a,T> Iterator for Same<'a,T>{type Item=&'a mut T;fn next(&mut self)->Option<&'a mut T>{self.items.iter_mut().next()}} fn main(){}",
                "lifetime may not live long enough",
            ),
            invalid(
                "unused_iterator_restarts_slice_iter_mut",
                "struct Slot<'a,T>{items:&'a mut [T]} impl<'a,T> Iterator for Slot<'a,T>{type Item=&'a mut T;fn next(&mut self)->Option<&'a mut T>{self.items.iter_mut().next()}} fn main(){}",
                "lifetime may not live long enough",
            ),
            // A shared slice returned out of an exclusive borrow keeps that
            // borrow exclusive for as long as the result is live.
            invalid(
                "shared_result_of_mut_vec_blocks_push",
                "fn f<'a,T>(v:&'a mut Vec<T>)->&'a [T]{v.as_slice()} fn main(){let mut v=vec![1,2,3];let s=f(&mut v);v.push(4);println!(\"{}\",s.len());}",
                "E0499",
            ),
            invalid(
                "shared_result_of_mut_vec_blocks_len",
                "fn f<'a,T>(v:&'a mut Vec<T>)->&'a [T]{v.as_slice()} fn main(){let mut v=vec![1,2,3];let s=f(&mut v);let n=v.len();println!(\"{} {}\",s.len(),n);}",
                "E0502",
            ),
            // Stock reports E0506 here. We reject the same write as E0502.
            invalid(
                "shared_result_of_mut_slice_blocks_write",
                "fn g<'a,T>(v:&'a mut [T])->&'a [T]{v} fn main(){let mut a=[1,2,3];let s=g(&mut a);a[0]=9;println!(\"{}\",s.len());}",
                "E0506",
            ),
            Case {
                name: "wrapped_mut_vec_blocks_len",
                source: r#"struct View<'a,T>{s:&'a[T]}
fn vec_view<'a,T>(v:&'a mut Vec<T>)->View<'a,T>{View{s:v.as_slice()}}
fn slice_view<'a,T>(v:&'a mut[T])->View<'a,T>{View{s:&*v}}
fn main(){let mut v:Vec<i32>=vec![1,2];let w=vec_view(&mut v);let n=v.len();println!("{} {}",w.s.len(),n);}"#,
                expect: Expect::Invalid("E0502"),
            },
            Case {
                name: "wrapped_mut_vec_blocks_push",
                source: r#"struct View<'a,T>{s:&'a[T]}
fn vec_view<'a,T>(v:&'a mut Vec<T>)->View<'a,T>{View{s:v.as_slice()}}
fn slice_view<'a,T>(v:&'a mut[T])->View<'a,T>{View{s:&*v}}
fn main(){let mut v:Vec<i32>=vec![1,2];let w=vec_view(&mut v);v.push(3);println!("{}",w.s.len());}"#,
                expect: Expect::Invalid("E0499"),
            },
            Case {
                name: "wrapped_mut_slice_blocks_len",
                source: r#"struct View<'a,T>{s:&'a[T]}
fn vec_view<'a,T>(v:&'a mut Vec<T>)->View<'a,T>{View{s:v.as_slice()}}
fn slice_view<'a,T>(v:&'a mut[T])->View<'a,T>{View{s:&*v}}
fn main(){let mut a:[i32;2]=[1,2];let s:&mut[i32]=&mut a;let w=slice_view(s);let n=s.len();println!("{} {}",w.s.len(),n);}"#,
                expect: Expect::Invalid("E0502"),
            },
            // Stock reports E0503/E0506 for this write. We report E0502.
            Case {
                name: "wrapped_mut_slice_blocks_write",
                source: r#"struct View<'a,T>{s:&'a[T]}
fn vec_view<'a,T>(v:&'a mut Vec<T>)->View<'a,T>{View{s:v.as_slice()}}
fn slice_view<'a,T>(v:&'a mut[T])->View<'a,T>{View{s:&*v}}
fn main(){let mut a:[i32;2]=[1,2];let s:&mut[i32]=&mut a;let w=slice_view(s);s[0]=3;println!("{}",w.s.len());}"#,
                expect: Expect::Invalid("E0503"),
            },
            Case {
                name: "wrapped_mut_vec_released_after_last_use",
                source: r#"struct View<'a,T>{s:&'a[T]}
fn vec_view<'a,T>(v:&'a mut Vec<T>)->View<'a,T>{View{s:v.as_slice()}}
fn slice_view<'a,T>(v:&'a mut[T])->View<'a,T>{View{s:&*v}}
fn main(){let mut v:Vec<i32>=vec![1,2];let w=vec_view(&mut v);println!("{}",w.s.len());v.push(3);println!("{:?}",v);}"#,
                expect: Expect::Run("2\n[1, 2, 3]\n"),
            },
            Case {
                name: "wrapped_mut_slice_released_after_last_use",
                source: r#"struct View<'a,T>{s:&'a[T]}
fn vec_view<'a,T>(v:&'a mut Vec<T>)->View<'a,T>{View{s:v.as_slice()}}
fn slice_view<'a,T>(v:&'a mut[T])->View<'a,T>{View{s:&*v}}
fn main(){let mut a:[i32;2]=[1,2];let s:&mut[i32]=&mut a;let w=slice_view(s);println!("{}",w.s.len());s[0]=3;println!("{}",s[0]);}"#,
                expect: Expect::Run("2\n3\n"),
            },
            invalid(
                "shared_result_of_mut_slice_blocks_len",
                "fn g<'a,T>(v:&'a mut [T])->&'a [T]{v} fn main(){let mut a=[1,2,3];let s=g(&mut a);let n=a.len();println!(\"{} {}\",s.len(),n);}",
                "E0502",
            ),
        ],
    );
}

#[test]
fn unchanged_m2_lifetimes_borrowing_matches_stock() {
    #[path = "support/closure_oracle.rs"]
    mod closure_oracle;
    closure_oracle::check(
        "unchanged-m2-lifetimes-borrowing",
        &[closure_oracle::Case {
            name: "11_lifetimes_borrowing",
            source: include_str!("milestones/m2/11_lifetimes_borrowing.rs"),
            expect: closure_oracle::Expect::Run(include_str!(
                "milestones/m2/11_lifetimes_borrowing.stdout"
            )),
        }],
    );
}
