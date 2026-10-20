//! Associated constants retain declaration identity and are checked when unused.
//! Value-use cases are added alongside the graph member-use producer.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/oracle.rs"]
mod oracle;
use oracle::{Case, Expect};

fn check(tag: &str, cases: &[Case]) {
    oracle::check(tag, cases);
    struct Scratch(std::path::PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let root = std::env::var_os("JCODE_SCRATCH_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join(format!("jr-associated-modes-{}-{tag}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let scratch = Scratch(root);
    for (index, case) in cases.iter().enumerate() {
        let source = scratch.0.join(format!("case-{index}.rs"));
        std::fs::write(&source, case.source).unwrap();
        for workers in ["1", "8"] {
            for check_only in [true, false] {
                let image = scratch.0.join(format!(
                    "case-{index}-workers-{workers}-{}",
                    if check_only { "check" } else { "build" }
                ));
                let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_jr-rust"));
                command
                    .arg(&source)
                    .args(["--workers", workers])
                    .current_dir(&scratch.0);
                if check_only {
                    command.arg("--check");
                } else {
                    command.arg("-o").arg(&image);
                }
                let mut before = std::fs::read_dir(&scratch.0)
                    .unwrap()
                    .map(|entry| entry.unwrap().path())
                    .collect::<Vec<_>>();
                before.sort();
                let output = command.output().unwrap();
                let diagnostic = String::from_utf8_lossy(&output.stderr);
                let label = format!("{} workers={workers} check={check_only}", case.name);
                assert!(!diagnostic.contains("panicked at"), "{label}: {diagnostic}");
                match case.expect {
                    Expect::Run(stdout) => {
                        assert!(output.status.success(), "{label}: {diagnostic}");
                        if !check_only {
                            let ran = std::process::Command::new(&image).output().unwrap();
                            assert!(ran.status.success(), "{label}: {ran:?}");
                            assert_eq!(ran.stdout, stdout.as_bytes(), "{label}: {ran:?}");
                        }
                    }
                    Expect::Invalid(_) => {
                        assert!(!output.status.success(), "{label}: accepted invalid source");
                        assert!(!diagnostic.contains("unsupported"), "{label}: {diagnostic}");
                    }
                }
                if check_only {
                    let mut after = std::fs::read_dir(&scratch.0)
                        .unwrap()
                        .map(|entry| entry.unwrap().path())
                        .collect::<Vec<_>>();
                    after.sort();
                    assert_eq!(before, after, "{label}: CHECK created an artifact");
                }
                if check_only || matches!(case.expect, Expect::Invalid(_)) {
                    assert!(!image.exists(), "{label}: wrote an executable");
                }
            }
        }
    }
}

#[test]
fn primitive_i32_min_source_and_alias_match_stock() {
    check(
        "primitive-i32-min",
        &[
            Case {
                name: "direct_and_alias_use_same_primitive_declaration",
                source: "type Number=i32;fn main(){let direct:i32=i32::MIN;let alias:i32=Number::MIN;println!(\"{} {}\",direct,alias);}",
                expect: Expect::Run("-2147483648 -2147483648\n"),
            },
            Case {
                name: "user_primitive_impl_has_no_loader_issuer",
                source: "impl i32{const MIN:i32=0;}fn main(){}",
                expect: Expect::Invalid("E0390"),
            },
            Case {
                name: "unused_bad_body_is_not_hidden_by_min_selection",
                source: "fn unused()->u8{true}fn main(){let value:i32=i32::MIN;}",
                expect: Expect::Invalid("E0308"),
            },
        ],
    );
}

#[test]
fn associated_initializer_member_dependencies_match_stock() {
    check(
        "associated-initializer-dependencies",
        &[
            Case {
                name: "default_dependency_uses_override",
                source: "trait T{const BASE:u16=3;const VALUE:u16=Self::BASE+2;} impl T for u8{const BASE:u16=500;} impl T for u16{} fn read<X:T>()->u16{X::VALUE} fn main(){println!(\"{} {}\",read::<u8>(),read::<u16>());}",
                expect: Expect::Run("502 5\n"),
            },
            Case {
                name: "inherent_dependency_chain",
                source: "struct Unit;impl Unit{const A:u16=7;const B:u16=Self::A*2;const C:u16=Self::B+Self::A;}fn main(){println!(\"{}\",Unit::C);}",
                expect: Expect::Run("21\n"),
            },
            Case {
                name: "generic_inherent_dependency",
                source: "struct Wrap<T>{value:T}impl<T>Wrap<T>{const A:u16=11;const B:u16=Self::A+1;}fn read<T>()->u16{Wrap::<T>::B}fn main(){println!(\"{} {}\",read::<u8>(),read::<u16>());}",
                expect: Expect::Run("12 12\n"),
            },
            Case {
                name: "inherent_dependency_cycle",
                source: "struct Unit;impl Unit{const A:u16=Self::B;const B:u16=Self::A;}fn main(){let value=Unit::A;}",
                expect: Expect::Invalid("E0391"),
            },
            Case {
                name: "short_circuit_still_qualifies_member_cycle",
                source: "trait T{const CYCLE:bool=Self::CYCLE;const OK:bool=false&&Self::CYCLE;}impl T for u8{}fn main(){println!(\"{}\",<u8 as T>::OK);}",
                expect: Expect::Invalid("E0391"),
            },
            Case {
                name: "override_does_not_evaluate_cyclic_default",
                source: "trait T{const VALUE:u16=Self::VALUE;}impl T for u8{const VALUE:u16=9;}fn main(){println!(\"{}\",<u8 as T>::VALUE);}",
                expect: Expect::Run("9\n"),
            },
            Case {
                name: "ordered_owner_actuals_select_distinct_dependencies",
                source: "trait T{const N:u16;}impl T for u8{const N:u16=10;}impl T for u16{const N:u16=20;}struct Wrap<X>{value:X}impl<X:T>Wrap<X>{const VALUE:u16=X::N+1;}fn read<X:T>()->u16{Wrap::<X>::VALUE}fn main(){println!(\"{} {}\",read::<u8>(),read::<u16>());}",
                expect: Expect::Run("11 21\n"),
            },
            Case {
                name: "unused_member_input_type_error",
                source: "trait T{const FLAG:bool=true;const VALUE:u16=Self::FLAG;}fn main(){}",
                expect: Expect::Invalid("E0308"),
            },
        ],
    );
}

#[test]
fn inherent_constant_owners_and_bounds_match_stock() {
    check(
        "inherent-associated-constants",
        &[
            Case {
                name: "inherent_value_and_self",
                source: "struct Unit; impl Unit { const VALUE:u16=513; fn value()->u16{Self::VALUE} } fn main(){println!(\"{} {}\",Unit::VALUE,Unit::value());}",
                expect: Expect::Run("513 513\n"),
            },
            Case {
                name: "generic_inherent_owner",
                source: "struct Wrap<T>{value:T} impl<T> Wrap<T>{const VALUE:u16=11;} fn read<T>()->u16{Wrap::<T>::VALUE} fn main(){println!(\"{} {}\",read::<u8>(),read::<u16>());}",
                expect: Expect::Run("11 11\n"),
            },
            Case {
                name: "conditional_inherent_owner",
                source: "trait Flag{} impl Flag for u8{} struct Wrap<T>{value:T} impl<T:Flag> Wrap<T>{const VALUE:u8=9;} fn main(){println!(\"{}\",Wrap::<u8>::VALUE);}",
                expect: Expect::Run("9\n"),
            },
            Case {
                name: "unsatisfied_inherent_owner",
                source: "trait Flag{} impl Flag for u8{} struct Wrap<T>{value:T} impl<T:Flag> Wrap<T>{const VALUE:u8=9;} fn main(){let x=Wrap::<u16>::VALUE;}",
                expect: Expect::Invalid("E0599"),
            },
        ],
    );
}

#[test]
fn associated_constant_declarations_and_unused_errors_match_stock() {
    check(
        "associated-constants",
        &[
            Case {
                name: "declarations_and_defaults",
                source: "trait T{const VALUE:u8=3;} impl T for i32{const VALUE:u8=7;} impl T for bool{} fn main(){println!(\"{}\",7i32);}",
                expect: Expect::Run("7\n"),
            },
            Case {
                name: "associated_output_declared_type",
                source: "trait Per<T>{type Output;const VALUE:Self::Output;} struct Unit{tag:u8} impl Per<Unit> for Unit{type Output=u8;const VALUE:u8=1;} fn main(){println!(\"{}\",1u8);}",
                expect: Expect::Run("1\n"),
            },
            Case {
                name: "qualified_override_and_default_values",
                source: "trait T{const VALUE:u8=3;} impl T for i32{const VALUE:u8=7;} impl T for bool{} fn main(){println!(\"{} {}\",<i32 as T>::VALUE,<bool as T>::VALUE);}",
                expect: Expect::Run("7 3\n"),
            },
            Case {
                name: "qualified_bool_and_arithmetic_values",
                source: "const BASE:i32=4;trait T{const N:i32=BASE*3+2;const YES:bool=true;} impl T for i32{} fn main(){println!(\"{} {}\",<i32 as T>::N,<i32 as T>::YES);}",
                expect: Expect::Run("14 true\n"),
            },
            Case {
                name: "generic_qualified_values",
                source: "trait T{const VALUE:u8=3;} impl T for i32{const VALUE:u8=7;} impl T for bool{} fn read<X:T>()->u8{<X as T>::VALUE} fn main(){println!(\"{} {}\",read::<i32>(),read::<bool>());}",
                expect: Expect::Run("7 3\n"),
            },
            Case {
                name: "generic_implicit_values",
                source: "trait T{const VALUE:u8=3;} impl T for i32{const VALUE:u8=7;} impl T for bool{} fn read<X:T>()->u8{X::VALUE} fn main(){println!(\"{} {}\",read::<i32>(),read::<bool>());}",
                expect: Expect::Run("7 3\n"),
            },
            Case {
                name: "self_implicit_associated_output",
                source: "trait Per<T>{type Output;const VALUE:Self::Output;} struct Unit; struct Larger; impl Per<Unit> for Unit{type Output=u8;const VALUE:u8=1;} impl Per<Larger> for Unit{type Output=u16;const VALUE:u16=1000;} impl Unit{fn per<T>(_v:T)-><Self as Per<T>>::Output where Self:Per<T>{Self::VALUE}} fn main(){println!(\"{} {}\",Unit::per(Unit),Unit::per(Larger));}",
                expect: Expect::Run("1 1000\n"),
            },
            Case {
                name: "unused_missing",
                source: "trait T{const VALUE:u8;} impl T for i32{} fn main(){}",
                expect: Expect::Invalid("E0046"),
            },
            Case {
                name: "unused_declared_type_mismatch",
                source: "trait T{const VALUE:u8;} impl T for i32{const VALUE:u16=1;} fn main(){}",
                expect: Expect::Invalid("E0326"),
            },
            Case {
                name: "unused_initializer_type_mismatch",
                source: "trait T{const VALUE:u8=true;} fn main(){}",
                expect: Expect::Invalid("E0308"),
            },
            Case {
                name: "unused_foreign_member",
                source: "trait T{const VALUE:u8=1;} impl T for i32{const OTHER:u8=2;} fn main(){}",
                expect: Expect::Invalid("E0438"),
            },
        ],
    );
}
