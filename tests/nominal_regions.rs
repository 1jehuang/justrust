//! Named nominal lifetimes must not be replaced by temporary receiver loans.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod oracle {
    use std::path::PathBuf;
    use std::process::Command;
    pub enum Expect {
        Run(&'static str),
        Invalid(&'static str),
    }
    pub struct Case {
        pub name: &'static str,
        pub source: &'static str,
        pub expect: Expect,
    }
    pub fn check(tag: &str, cases: &[Case]) {
        let root = std::env::var_os("JCODE_SCRATCH_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join(format!("jr-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let selected = std::env::var_os("JR_NOMINAL_REGIONS_COMPILER")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_jr-rust")));
        let native = root.join("compiler");
        std::fs::copy(&selected, &native).unwrap();
        let stock = std::env::var_os("JUSTRUST_REAL_RUSTC")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("rustc"));
        let mut failures = Vec::new();
        for (index, case) in cases.iter().enumerate() {
            let source = root.join(format!("case-{index}.rs"));
            std::fs::write(&source, case.source).unwrap();
            let stock_image = root.join(format!("stock-{index}"));
            let output = Command::new(&stock)
                .env_remove("RUSTC_BOOTSTRAP")
                .args([
                    "--edition=2024",
                    "-Cpanic=abort",
                    "--crate-name=oracle",
                    "-Awarnings",
                ])
                .arg(&source)
                .arg("-o")
                .arg(&stock_image)
                .output()
                .expect("stock compiler required");
            let stderr = String::from_utf8_lossy(&output.stderr);
            match case.expect {
                Expect::Run(expected) => {
                    assert!(
                        output.status.success(),
                        "{}: stock rejected: {stderr}",
                        case.name
                    );
                    let ran = Command::new(&stock_image).output().unwrap();
                    assert!(ran.status.success());
                    assert_eq!(
                        ran.stdout,
                        expected.as_bytes(),
                        "{} stock stdout",
                        case.name
                    );
                }
                Expect::Invalid(fragment) => assert!(
                    !output.status.success() && stderr.contains(fragment),
                    "{}: expected stock {fragment}: {stderr}",
                    case.name
                ),
            }
            for workers in [1, 8] {
                for check in [true, false] {
                    let image = root.join(format!("native-{index}-{workers}"));
                    let mut command = Command::new(&native);
                    command
                        .arg(&source)
                        .arg("--workers")
                        .arg(workers.to_string());
                    if check {
                        command.arg("--check");
                    } else {
                        command.arg("-o").arg(&image);
                    }
                    let output = command.output().unwrap();
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    let label = format!("{} workers={workers} check={check}", case.name);
                    match case.expect {
                        Expect::Run(expected) => {
                            if !output.status.success() {
                                failures.push(format!("{label}: {stderr}"));
                                continue;
                            }
                            if !check {
                                let ran = Command::new(&image).output().unwrap();
                                if !ran.status.success() || ran.stdout != expected.as_bytes() {
                                    failures.push(format!("{label}: run {ran:?}"));
                                }
                            }
                        }
                        Expect::Invalid(_) => {
                            if output.status.success()
                                || stderr.to_ascii_lowercase().contains("unsupported")
                            {
                                failures.push(format!(
                                    "{label}: invalid acceptance/boundary: {stderr}"
                                ));
                            }
                        }
                    }
                }
            }
        }
        if failures.is_empty() {
            std::fs::remove_dir_all(&root).unwrap();
        }
        assert!(
            failures.is_empty(),
            "{} of {} cases failed:\n{}",
            failures.len(),
            cases.len(),
            failures.join("\n")
        );
    }
}
use oracle::{Case, Expect};

#[test]
fn named_carrier_preserves_exact_field_return_paths() {
    oracle::check(
        "nominal-field-paths",
        &[Case {
            name: "named_receiver_and_stored_carriers_keep_owned_field_suffix",
            source: r#"struct S<'a> { r: &'a i32, own: i32 }
impl<'a> S<'a> {
 fn own<'s>(&'s self) -> &'s i32 { &self.own }
 fn indirect(&self) -> &i32 { self.own() }
}
fn main() { let x=5; let s=S { r:&x, own:7 }; let result=*s.indirect(); println!("{}",result); }"#,
            expect: Expect::Run("7\n"),
        }],
    );
}

#[test]
fn generic_nominal_carrier_regions_match_stock() {
    oracle::check(
        "generic-nominal-regions",
        &[
            Case {
                name: "generic_nominal_carrier_transport",
                source: r#"struct W<'a,T> { r: &'a T }
impl<'a,T> W<'a,T> { fn get(&self) -> &'a T { self.r } }
fn main() { let a=11; let x:&i32; { let w=W{r:&a}; x=w.get(); } println!("{}",x); }"#,
                expect: Expect::Run("11\n"),
            },
            Case {
                name: "generic_nominal_distinct_region_escape_rejected",
                source: r#"struct W<'a,T> { r: &'a T }
fn get<'a,'b,T>(w:W<'b,T>) -> &'a T { w.r }
fn main() {}"#,
                expect: Expect::Invalid("lifetime"),
            },
        ],
    );
}

#[test]
fn nominal_carrier_regions_match_stock() {
    oracle::check(
        "nominal-regions",
        &[
            Case {
                name: "shared_item_outlives_wrapper",
                source: r#"struct W<'a> { r: &'a i32 }
impl<'a> W<'a> { fn get(&self) -> &'a i32 { self.r } }
fn main() { let a = 7; let x: &i32; { let w = W { r: &a }; x = w.get(); } println!("{}", x); }"#,
                expect: Expect::Run("7\n"),
            },
            Case {
                name: "two_next_results_outlive_iterator",
                source: r#"struct I<'a> { a: &'a [i32; 2], n: usize }
impl<'a> I<'a> { fn next(&mut self) -> Option<&'a i32> { if self.n < 2 { let i = self.n; self.n += 1; Some(&self.a[i]) } else { None } } }
fn main() { let a = [7, 9]; let x: Option<&i32>; let y: Option<&i32>; { let mut i = I { a: &a, n: 0 }; x = i.next(); y = i.next(); } match x { Some(x) => println!("{}", x), None => () } match y { Some(y) => println!("{}", y), None => () } }"#,
                expect: Expect::Run("7\n9\n"),
            },
            Case {
                name: "associated_iterator_item_outlives_receiver",
                source: r#"trait Next { type Item; fn next(&mut self) -> Option<Self::Item>; }
struct I<'a> { a: &'a [i32;2], n: usize }
impl<'a> Next for I<'a> { type Item = &'a i32; fn next(&mut self) -> Option<Self::Item> { if self.n < 2 { let n=self.n; self.n += 1; Some(&self.a[n]) } else { None } } }
fn main() { let a=[3,8]; let x:Option<&i32>; let y:Option<&i32>; { let mut i=I{a:&a,n:0}; x=i.next(); y=i.next(); } match x {Some(x)=>println!("{}",x),None=>()} match y {Some(y)=>println!("{}",y),None=>()} }"#,
                expect: Expect::Run("3\n8\n"),
            },
            Case {
                name: "ordered_distinct_nominal_regions",
                source: r#"struct W<'a, 'b> { a: &'a i32, b: &'b i32 }
impl<'a, 'b> W<'a, 'b> { fn first(&self) -> &'a i32 { self.a } fn second(&self) -> &'b i32 { self.b } }
fn main() { let a = 3; let b = 8; let w = W { a: &a, b: &b }; println!("{} {}", w.first(), w.second()); }"#,
                expect: Expect::Run("3 8\n"),
            },
            Case {
                name: "own_field_cannot_escape_as_nominal_region",
                source: r#"struct W<'a> { r: &'a i32, own: i32 }
impl<'a> W<'a> { fn get(&self) -> &'a i32 { &self.own } }
fn main() {}"#,
                expect: Expect::Invalid("lifetime"),
            },
            Case {
                name: "option_own_field_cannot_escape",
                source: r#"struct W<'a> { r: &'a i32, own: i32 }
impl<'a> W<'a> { fn get(&mut self) -> Option<&'a i32> { Some(&self.own) } }
fn main() {}"#,
                expect: Expect::Invalid("lifetime"),
            },
            Case {
                name: "mutable_reborrow_cannot_outlive_receiver",
                source: r#"struct W<'a> { r: &'a mut i32 }
impl<'a> W<'a> { fn get(&mut self) -> &'a mut i32 { &mut *self.r } }
fn main() {}"#,
                expect: Expect::Invalid("lifetime"),
            },
            Case {
                name: "distinct_regions_cannot_be_swapped",
                source: r#"struct W<'a, 'b> { a: &'a i32, b: &'b i32 }
impl<'a, 'b> W<'a, 'b> { fn first(&self) -> &'a i32 { self.b } }
fn main() {}"#,
                expect: Expect::Invalid("lifetime"),
            },
            Case {
                name: "replace_same_nominal_lifetime",
                source: r#"struct W<'a> { r: &'a i32 }
fn replace<'a>(w: &mut W<'a>, r: &'a i32) { w.r = r; }
fn main() { let a=3; let b=7; let mut w=W{r:&a}; replace(&mut w,&b); println!("{}",*w.r); }"#,
                expect: Expect::Run("7\n"),
            },
            Case {
                name: "shorter_field_replacement_rejected",
                source: r#"struct W<'a> { r: &'a i32 }
fn replace<'a,'b>(w: &mut W<'a>, r: &'b i32) { w.r = r; }
fn main() {}"#,
                expect: Expect::Invalid("lifetime"),
            },
            Case {
                name: "referent_still_cannot_dangle",
                source: r#"struct W<'a> { r: &'a i32 }
impl<'a> W<'a> { fn get(&self) -> &'a i32 { self.r } }
fn main() { let x: &i32; { let a = 7; let w = W { r: &a }; x = w.get(); } println!("{}", x); }"#,
                expect: Expect::Invalid("E0597"),
            },
        ],
    );
}
