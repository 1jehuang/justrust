//! Inferred local types must not create source-level reborrow coercion sites.
//! Partial nominal applications (`Wrap::<_>(r)` and `Wrap::<&mut _>(r)`)
//! currently have a separate explicit unsupported diagnostic in the generic
//! application dispatcher. They are not counted as E0382 authority coverage.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/oracle.rs"]
mod oracle;

#[test]
fn inferred_reference_bindings_preserve_moves() {
    let sources = [
        ("plain", "fn unused(r:&mut i32)->i32{let q=r;*r}fn main(){}"),
        (
            "immediate_closure",
            "fn unused(mut n:i32){(||->i32{let r=&mut n;let q=r;*r})();}fn main(){}",
        ),
        (
            "tuple",
            "fn unused(r:&mut i32)->i32{let q=(r,);*r}fn main(){}",
        ),
        (
            "array",
            "fn unused(r:&mut i32)->i32{let q=[r];*r}fn main(){}",
        ),
        (
            "block",
            "fn unused(r:&mut i32)->i32{let q={r};*r}fn main(){}",
        ),
        (
            "branch",
            "fn unused(r:&mut i32)->i32{let q=if true{r}else{r};*r}fn main(){}",
        ),
        (
            "match",
            "fn unused(r:&mut i32)->i32{let q=match true{true=>r,false=>r};*r}fn main(){}",
        ),
        (
            "generic_option",
            "fn unused(r:&mut i32)->i32{let q=Some(r);*r}fn main(){}",
        ),
        (
            "generic_tuple_constructor",
            "struct Wrap<T>(T);fn unused(r:&mut i32)->i32{let q=Wrap(r);*r}fn main(){}",
        ),
        (
            "generic_record_constructor",
            "struct Wrap<T>{value:T}fn unused(r:&mut i32)->i32{let q=Wrap{value:r};*r}fn main(){}",
        ),
        (
            "nested_generic_constructor",
            "struct Wrap<T>{value:Option<T>}fn unused(r:&mut i32)->i32{let q=Wrap{value:Some(r)};*r}fn main(){}",
        ),
        (
            "loop",
            "fn unused(r:&mut i32)->i32{let q=loop{break r;};*r}fn main(){}",
        ),
    ];
    let cases: Vec<_> = sources
        .iter()
        .map(|(name, source)| oracle::Case {
            name,
            source,
            expect: oracle::Expect::Invalid("E0382"),
        })
        .collect();
    oracle::check("inference-reference-moves", &cases);
    for (_, source) in sources {
        for workers in [1, 8] {
            for check_only in [true, false] {
                let failure =
                    justrust_engine::compiler::compile_observed(source.into(), workers, check_only)
                        .err()
                        .expect("an inferred binding must preserve the unique-reference move");
                let message = failure.to_string();
                assert!(!message.contains("unsupported"), "{message}\n{source}");
                assert!(!message.contains("panicked"), "{message}\n{source}");
            }
        }
    }
}

#[test]
fn written_reference_expectations_still_reborrow() {
    oracle::check(
        "written-reference-reborrows",
        &[
            oracle::Case {
                name: "typed_binding",
                source: "fn main(){let mut n=1i32;let r=&mut n;let q:&mut i32=r;*q+=1;*r+=2;println!(\"{}\",*r);}",
                expect: oracle::Expect::Run("4\n"),
            },
            oracle::Case {
                name: "written_parameter",
                source: "fn bump(r:&mut i32){*r+=1;}fn main(){let mut n=1i32;let r=&mut n;bump(r);*r+=2;println!(\"{}\",*r);}",
                expect: oracle::Expect::Run("4\n"),
            },
            oracle::Case {
                name: "written_constructor_binding",
                source: "struct Wrap<T>(T);fn main(){let mut n=1i32;let r=&mut n;let q:Wrap<&mut i32>=Wrap(r);*q.0+=1;*r+=2;println!(\"{}\",*r);}",
                expect: oracle::Expect::Run("4\n"),
            },
            oracle::Case {
                name: "written_constructor_actual",
                source: "struct Wrap<T>(T);fn main(){let mut n=1i32;let r=&mut n;let q=Wrap::<&mut i32>(r);*q.0+=1;*r+=2;println!(\"{}\",*r);}",
                expect: oracle::Expect::Run("4\n"),
            },
            oracle::Case {
                name: "declared_reference_field",
                source: "struct Wrap<'a>{value:&'a mut i32}fn main(){let mut n=1i32;let r=&mut n;let q=Wrap{value:r};*q.value+=1;*r+=2;println!(\"{}\",*r);}",
                expect: oracle::Expect::Run("4\n"),
            },
            oracle::Case {
                name: "canonical_reference_identity",
                source: "fn identity<T>(x:T)->T{x}fn main(){let value=42i32;let r=identity(&value);println!(\"{}\",*r);}",
                expect: oracle::Expect::Run("42\n"),
            },
        ],
    );
}

#[test]
fn unit_and_shift_constraints_keep_rust_type_shapes() {
    oracle::check(
        "inference-unit-and-shifts",
        &[
            oracle::Case {
                name: "unit_pattern",
                source: "fn f(u:()){let ()=u;let real=u;}fn main(){f(());println!(\"unit\");}",
                expect: oracle::Expect::Run("unit\n"),
            },
            oracle::Case {
                name: "independent_indexed_shift_rhs",
                source: "fn main(){let mut a=[9i8,7i8];let i=0usize;a[i]<<=1u64;a[i]>>=1i8;println!(\"{} {}\",a[0],a[1]);}",
                expect: oracle::Expect::Run("9 7\n"),
            },
        ],
    );
}
