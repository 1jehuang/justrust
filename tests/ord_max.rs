//! Genuine Ord source defaults, separate from first-class function items.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/closure_oracle.rs"]
mod oracle;
use oracle::{Case, Expect};

#[test]
fn ord_clamp_reversed_bounds_panic_before_returning() {
    use std::{
        fs,
        os::unix::{fs::PermissionsExt, process::ExitStatusExt},
        process::Command,
    };
    let root = std::env::var_os("JCODE_SCRATCH_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join(format!("ord-clamp-abort-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let source = r#"fn main(){let value=3i32.clamp(5,1);println!("returned {}",value);}"#;
    fs::write(root.join("main.rs"), source).unwrap();
    let stock =
        Command::new(std::env::var_os("JUSTRUST_REAL_RUSTC").unwrap_or_else(|| "rustc".into()))
            .args(["--edition=2024", "-Cpanic=abort", "--crate-name=oracle"])
            .arg(root.join("main.rs"))
            .arg("-o")
            .arg(root.join("stock"))
            .output()
            .unwrap();
    assert!(
        stock.status.success(),
        "{}",
        String::from_utf8_lossy(&stock.stderr)
    );
    let expected = Command::new(root.join("stock"))
        .current_dir(&root)
        .output()
        .unwrap();
    assert_eq!(expected.status.signal(), Some(6));
    assert!(expected.stdout.is_empty());
    for workers in [1, 8] {
        for check in [true, false] {
            let compiled =
                justrust_engine::compiler::compile(source.into(), workers, check).unwrap();
            if check {
                assert!(compiled.executable.is_none());
                assert!(
                    compiled
                        .report
                        .node_evaluations
                        .iter()
                        .all(|(key, _)| !matches!(key.kind, 103 | 105 | 130 | 131 | 133))
                );
                continue;
            }
            let image = root.join(format!("native-{workers}"));
            fs::write(&image, compiled.executable.unwrap()).unwrap();
            fs::set_permissions(&image, fs::Permissions::from_mode(0o700)).unwrap();
            let actual = Command::new(&image).current_dir(&root).output().unwrap();
            assert_eq!(actual.status.signal(), expected.status.signal());
            assert_eq!(actual.stdout, expected.stdout);
            assert!(!actual.stderr.is_empty());
        }
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn ord_max_method_ufcs_alias_and_generic_match_stock() {
    oracle::check(
        "ord-max-scalar",
        &[Case {
            name: "canonical_default_calls",
            source: "type Number=i32;fn choose<T:Ord>(a:T,b:T)->T{a.max(b)}fn main(){let a=3i32.max(7);let b=i32::max(9,4);let c=Number::max(-5,-2);let d=<i32 as Ord>::max(6,6);let e=choose(11i32,8i32);println!(\"{} {} {} {} {}\",a,b,c,d,e);}",
            expect: Expect::Run("7 9 -2 6 11\n"),
        }],
    );
}

#[test]
fn ord_min_clamp_method_ufcs_alias_and_generic_match_stock() {
    oracle::check(
        "ord-min-clamp",
        &[Case {
            name: "canonical_min_clamp_calls",
            source: r#"type Number=i32;fn choose<T:Ord>(a:T,b:T)->T{a.min(b)}fn bound<T:Ord>(a:T,lo:T,hi:T)->T{a.clamp(lo,hi)}fn main(){println!("{} {} {} {} {} {} {} {}",3i32.min(7),i32::min(9,4),Number::min(-5,-2),<i32 as Ord>::min(6,6),choose(11i32,8i32),bound(-2i32,0,5),Number::clamp(9,0,5),<i32 as Ord>::clamp(3,0,5));}"#,
            expect: Expect::Run("3 4 -5 6 8 0 5 3\n"),
        }],
    );
}

macro_rules! equal_values {
    ($body:literal) => {
        concat!(
            "use std::cmp::Ordering;struct Equal{tag:i32}impl PartialEq for Equal{fn eq(&self,other:&Self)->bool{true}}impl Eq for Equal{}impl PartialOrd for Equal{fn partial_cmp(&self,other:&Self)->Option<Ordering>{Some(Ordering::Equal)}}",
            $body
        )
    };
}

#[test]
fn ord_min_clamp_noncopy_ties_and_overrides_match_stock() {
    oracle::check(
        "ord-min-clamp-noncopy",
        &[
            Case {
                name: "equal_values_preserve_receiver",
                source: equal_values!(
                    "impl Ord for Equal{fn cmp(&self,other:&Self)->Ordering{Ordering::Equal}}fn main(){let a=Equal{tag:1}.min(Equal{tag:2});let b=Equal{tag:3}.clamp(Equal{tag:4},Equal{tag:5});println!(\"{} {}\",a.tag,b.tag);}"
                ),
                expect: Expect::Run("1 3\n"),
            },
            Case {
                name: "min_observable_comparator_order",
                source: r#"use std::cmp::Ordering;struct Value{key:i32,tag:i32}impl PartialEq for Value{fn eq(&self,other:&Self)->bool{self.key==other.key}}impl Eq for Value{}impl PartialOrd for Value{fn partial_cmp(&self,other:&Self)->Option<Ordering>{println!("partial {} {}",self.tag,other.tag);Some(self.cmp(other))}}impl Ord for Value{fn cmp(&self,other:&Self)->Ordering{println!("cmp {} {}",self.tag,other.tag);self.key.cmp(&other.key)}}fn main(){let chosen=Value{key:2,tag:1}.min(Value{key:1,tag:2});println!("chosen {}",chosen.tag);}"#,
                expect: Expect::Run("partial 2 1\ncmp 2 1\nchosen 2\n"),
            },
            Case {
                name: "canonical_min_clamp_overrides",
                source: equal_values!(
                    "impl Ord for Equal{fn cmp(&self,other:&Self)->Ordering{Ordering::Equal}fn min(self,other:Self)->Self{other}fn clamp(self,lo:Self,hi:Self)->Self{hi}}fn main(){let a=Equal{tag:1}.min(Equal{tag:2});let b=<Equal as Ord>::clamp(Equal{tag:3},Equal{tag:4},Equal{tag:5});println!(\"{} {}\",a.tag,b.tag);}"
                ),
                expect: Expect::Run("2 5\n"),
            },
        ],
    );
}

#[test]
fn ord_min_clamp_invalid_membership_types_moves_and_unused_bodies_reject() {
    oracle::check(
        "ord-min-clamp-invalid",
        &[
            Case {
                name: "missing_min_membership",
                source: "struct Value;fn main(){let x=Value.min(Value);}",
                expect: Expect::Invalid("E0599"),
            },
            Case {
                name: "missing_clamp_membership",
                source: "struct Value;fn main(){let x=Value.clamp(Value,Value);}",
                expect: Expect::Invalid("E0599"),
            },
            Case {
                name: "min_wrong_argument",
                source: "fn main(){let x=i32::min(1,true);}",
                expect: Expect::Invalid("E0308"),
            },
            Case {
                name: "clamp_wrong_bound",
                source: "fn main(){let x=1i32.clamp(0,true);}",
                expect: Expect::Invalid("E0308"),
            },
            Case {
                name: "unused_bad_body",
                source: "fn unused()->bool{3i32.min(4)}fn main(){let x=2i32.clamp(1,3);}",
                expect: Expect::Invalid("E0308"),
            },
            Case {
                name: "foreign_min_is_not_ord",
                source: "struct Value;trait Other{fn min(self,other:Self)->Self;}impl Other for Value{fn min(self,other:Self)->Self{self}}fn main(){let x=<Value as Ord>::min(Value,Value);}",
                expect: Expect::Invalid("E0277"),
            },
            Case {
                name: "min_moves_receiver",
                source: equal_values!(
                    "impl Ord for Equal{fn cmp(&self,other:&Self)->Ordering{Ordering::Equal}}fn main(){let a=Equal{tag:1};let b=a.min(Equal{tag:2});println!(\"{}\",a.tag);}"
                ),
                expect: Expect::Invalid("E0382"),
            },
            Case {
                name: "clamp_moves_bounds",
                source: equal_values!(
                    "impl Ord for Equal{fn cmp(&self,other:&Self)->Ordering{Ordering::Equal}}fn main(){let lo=Equal{tag:1};let a=Equal{tag:2}.clamp(lo,Equal{tag:3});println!(\"{}\",lo.tag);}"
                ),
                expect: Expect::Invalid("E0382"),
            },
        ],
    );
}

#[test]
fn ord_max_equal_noncopy_values_choose_second() {
    oracle::check(
        "ord-max-tie",
        &[Case {
            name: "noncopy_equal_second",
            source: equal_values!(
                "impl Ord for Equal{fn cmp(&self,other:&Self)->Ordering{Ordering::Equal}}fn main(){let chosen=Equal{tag:1}.max(Equal{tag:2});println!(\"{}\",chosen.tag);}"
            ),
            expect: Expect::Run("2\n"),
        }],
    );
}

#[test]
fn ord_max_user_override_is_canonical() {
    oracle::check(
        "ord-max-override",
        &[Case {
            name: "override_not_default",
            source: equal_values!(
                "impl Ord for Equal{fn cmp(&self,other:&Self)->Ordering{Ordering::Equal}fn max(self,other:Self)->Self{self}}fn main(){let chosen=<Equal as Ord>::max(Equal{tag:1},Equal{tag:2});println!(\"{}\",chosen.tag);}"
            ),
            expect: Expect::Run("1\n"),
        }],
    );
}

#[test]
fn ord_max_move_and_unrelated_spelling_do_not_bypass_authority() {
    oracle::check(
        "ord-max-authority",
        &[
            Case {
                name: "noncopy_receiver_is_moved",
                source: equal_values!(
                    "impl Ord for Equal{fn cmp(&self,other:&Self)->Ordering{Ordering::Equal}}fn main(){let first=Equal{tag:1};let chosen=first.max(Equal{tag:2});println!(\"{} {}\",first.tag,chosen.tag);}"
                ),
                expect: Expect::Invalid("E0382"),
            },
            Case {
                name: "unrelated_max_does_not_prove_ord",
                source: "struct Value;trait Other{fn max(self,other:Self)->Self;}impl Other for Value{fn max(self,other:Self)->Self{self}}fn main(){let value=<Value as Ord>::max(Value,Value);}",
                expect: Expect::Invalid("E0277"),
            },
        ],
    );
}

#[test]
fn ord_max_invalid_membership_arguments_and_unused_body_reject() {
    oracle::check(
        "ord-max-invalid",
        &[
            Case {
                name: "missing_ord",
                source: "struct Value;fn main(){let value=Value.max(Value);}",
                expect: Expect::Invalid("E0599"),
            },
            Case {
                name: "wrong_argument_type",
                source: "fn main(){let value=i32::max(1,true);}",
                expect: Expect::Invalid("E0308"),
            },
            Case {
                name: "unused_bad_body",
                source: "fn unused()->u8{true}fn main(){let value=1i32.max(2);}",
                expect: Expect::Invalid("E0308"),
            },
        ],
    );
}
