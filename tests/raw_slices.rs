//! Canonical raw-parts construction. See RAW-SLICES.md.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
use justrust_engine::compiler;
#[path = "support/closure_oracle.rs"]
mod closure_oracle;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[test]
fn bare_slice_actuals_preserve_authoritative_sized_obligations() {
    use closure_oracle::{Case, Expect};
    // Exercise binder obligations without introducing &T-to-slice carrier
    // transport, which remains a separate unsupported boundary.
    let cases = [
        Case {
            name: "maybe_unsized_slice_actual",
            source: "fn admit<T:?Sized>()->usize{7}fn main(){println!(\"{}\",admit::<[i32]>());}",
            expect: Expect::Run("7\n"),
        },
        Case {
            name: "implicit_sized_slice_actual_rejected",
            source: "fn need<T>(){}fn main(){need::<[i32]>();}",
            expect: Expect::Invalid("E0277"),
        },
        Case {
            name: "unused_implicit_sized_slice_actual_rejected",
            source: "fn need<T>(){}fn unused(){need::<[i32]>();}fn main(){}",
            expect: Expect::Invalid("E0277"),
        },
    ];
    closure_oracle::check("bare-slice-sized-obligations", &cases);
    // A resource or unsupported failure is not evidence for implicit Sized.
    for case in &cases[1..] {
        for workers in [1, 8] {
            for check in [true, false] {
                let failure = match compiler::compile_observed(case.source.into(), workers, check) {
                    Ok(_) => panic!("{}: invalid slice actual accepted", case.name),
                    Err(failure) => failure,
                };
                let diagnostic = failure.error.to_string();
                assert!(
                    failure.resource_usage.first_failure.is_none()
                        && !diagnostic.contains("unsupported")
                        && !diagnostic.contains("resource")
                        && !diagnostic.contains("limit exceeded"),
                    "{} workers={workers} check={check}: expected semantic Sized rejection: {diagnostic}",
                    case.name
                );
            }
        }
    }
}

#[test]
fn generic_mutable_slice_comparison_hints_match_stock() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "generic-mutable-index-hints",
        &[
            Case {
                name: "static_dynamic_generic_comparison",
                source: "fn below<T:PartialOrd+Copy>(v:&mut[T],p:T)->bool{let key=v[0];let i=1usize;v[i]<key&&v[0]<p}fn main(){let mut a=[3i32,1];println!(\"{}\",below(&mut a,5));let mut b=[1.25f64,-2.5];println!(\"{}\",below(&mut b,0.0));}",
                expect: Expect::Run("true\nfalse\n"),
            },
            Case {
                name: "comparison_requires_declared_bound",
                source: "fn bad<T:Copy>(v:&mut[T],p:T)->bool{v[0]<p}fn main(){}",
                expect: Expect::Invalid("E0369"),
            },
        ],
    );
}

#[test]
fn generic_range_reborrow_inference_matches_stock() {
    use closure_oracle::{Case, Expect};
    let cases = [
        Case {
            name: "mutable_range_evaluates_bounds_once",
            source: "fn set<T:Copy>(s:&mut[T],v:T){s[0]=v;}fn next(n:&mut usize)->usize{*n+=1;1}fn main(){let mut a=[1i32,2,3];let mut n=0usize;set(&mut a[next(&mut n)..],9);println!(\"{} {} {}\",a[0],a[1],n);}",
            expect: Expect::Run("1 9 1\n"),
        },
        Case {
            name: "generic_forwarding_and_shared_range",
            source: "fn set<T:Copy>(s:&mut[T],v:T){s[0]=v;}fn forward<T:Copy>(s:&mut[T],v:T){set(&mut s[1..],v);}fn first<T:Copy>(s:&[T])->T{s[0]}fn main(){let mut a=[1i32,2,3];forward(&mut a,8);println!(\"{} {}\",first(&a[1..]),a[2]);}",
            expect: Expect::Run("8 3\n"),
        },
        Case {
            name: "range_type_hint_does_not_grant_mutability",
            source: "fn set<T:Copy>(s:&mut[T],v:T){s[0]=v;}fn main(){let a=[1i32,2];set(&mut a[1..],9);}",
            expect: Expect::Invalid("E0596"),
        },
        Case {
            name: "range_type_hint_preserves_element_identity",
            source: "fn set<T:Copy>(s:&mut[T],v:T){s[0]=v;}fn main(){let mut a=[1i32,2];set::<bool>(&mut a[1..],true);}",
            expect: Expect::Invalid("E0308"),
        },
        Case {
            name: "range_type_hint_does_not_grant_overlapping_loans",
            source: "fn set<T:Copy>(s:&mut[T],v:T){s[0]=v;}fn main(){let mut a=[1i32,2];let r=&mut a[1..];set(&mut a[..],9);println!(\"{}\",r[0]);}",
            expect: Expect::Invalid("E0499"),
        },
    ];
    closure_oracle::check("generic-range-reborrow", &cases);
    for case in cases {
        if !matches!(case.expect, Expect::Invalid(_)) {
            continue;
        }
        for workers in [1, 8] {
            for check in [true, false] {
                let failure = match compiler::compile_observed(case.source.into(), workers, check) {
                    Ok(_) => panic!("{}: invalid range borrowing was accepted", case.name),
                    Err(failure) => failure,
                };
                let diagnostic = failure.error.to_string();
                assert!(
                    failure.resource_usage.first_failure.is_none()
                        && !diagnostic.contains("unsupported"),
                    "{} workers={workers} check={check}: {diagnostic}",
                    case.name
                );
            }
        }
    }
}

#[test]
fn generic_slice_deref_evidence_matches_stock() {
    use closure_oracle::{Case, Expect};
    let cases = [
        Case {
            name: "vec_elements_infer_through_shared_and_mutable_slice_formals",
            source: "fn first<T:Copy>(s:&[T])->T{s[0]}fn set<T:Copy>(s:&mut[T],v:T){s[0]=v;}fn main(){let mut v=vec![3i32,1];set(&mut v,7);println!(\"{}\",first(&v));let mut f=vec![1.5f64,2.0];set(&mut f,4.5);println!(\"{}\",first(&f));}",
            expect: Expect::Run("7\n4.5\n"),
        },
        Case {
            name: "local_vec_name_does_not_supply_deref_evidence",
            source: "struct Vec<T>{value:T}fn first<T:Copy>(s:&[T])->T{s[0]}fn main(){let v=Vec{value:1i32};println!(\"{}\",first(&v));}",
            expect: Expect::Invalid("E0308"),
        },
        Case {
            name: "deref_evidence_preserves_explicit_actual_identity",
            source: "fn first<T:Copy>(s:&[T])->T{s[0]}fn main(){let v=vec![1i32,2];let _=first::<bool>(&v);}",
            expect: Expect::Invalid("E0308"),
        },
        Case {
            name: "deref_evidence_does_not_grant_mutability",
            source: "fn set<T:Copy>(s:&mut[T],v:T){s[0]=v;}fn main(){let v=vec![1i32,2];set(&mut v,9);}",
            expect: Expect::Invalid("E0596"),
        },
    ];
    closure_oracle::check("generic-slice-deref-evidence", &cases);
    for case in &cases[1..] {
        for workers in [1, 8] {
            for check in [true, false] {
                let failure = match compiler::compile_observed(case.source.into(), workers, check) {
                    Ok(_) => panic!("{}: invalid slice coercion was accepted", case.name),
                    Err(failure) => failure,
                };
                assert!(
                    failure.resource_usage.first_failure.is_none()
                        && !failure.error.to_string().contains("unsupported"),
                    "{} workers={workers} check={check}: {}",
                    case.name,
                    failure.error
                );
            }
        }
    }
}

#[test]
fn generic_mutable_result_identity_matches_stock() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "generic-mutable-result-identity",
        &[
            Case {
                name: "generic_swap_loop_then_partition",
                source: "fn partition<T:Copy>(s:&mut[T]){let mut i=0usize;while i<s.len(){s.swap(i,i);i+=1;}let(l,r)=s.split_at_mut(1);l.swap(0,0);r.swap(0,0);}fn main(){let mut a=[3i32,2,1];partition(&mut a);println!(\"{} {} {}\",a[0],a[1],a[2]);let mut b=[1.5f64,2.5];partition(&mut b);println!(\"{} {}\",b[0],b[1]);}",
                expect: Expect::Run("3 2 1\n1.5 2.5\n"),
            },
            Case {
                name: "generic_recursive_partition_after_swap",
                source: "fn recurse<T:Copy>(s:&mut[T]){if s.len()<=1{return;}let n=s.len()-1;s.swap(0,n);let(l,r)=s.split_at_mut(1);recurse(l);recurse(&mut r[1..]);}fn main(){let mut a=[1i32,2,3,4];recurse(&mut a);println!(\"{} {} {} {}\",a[0],a[1],a[2],a[3]);}",
                expect: Expect::Run("4 2 1 3\n"),
            },
            Case {
                name: "generic_partition_descendant_loans",
                source: "fn split<T>(s:&mut[T])->(&mut[T],&mut[T]){s.split_at_mut(1)}fn work(s:&mut[i32]){let(l,r)=split(s);let a=&mut l[0];let b=&mut r[0];let old=*a;*a=*b;*b=old;}fn main(){let mut a=[1i32,2];work(&mut a);println!(\"{} {}\",a[0],a[1]);}",
                expect: Expect::Run("2 1\n"),
            },
            Case {
                name: "generic_partition_same_side_stays_exclusive",
                source: "fn split<T>(s:&mut[T])->(&mut[T],&mut[T]){s.split_at_mut(1)}fn bad(s:&mut[i32]){let(l,_)=split(s);let a=&mut l[0];let b=&mut l[0];*a=*b;}fn main(){}",
                expect: Expect::Invalid("E0499"),
            },
            Case {
                name: "distinct_calls_do_not_share_result_exclusivity",
                source: "fn split<T>(s:&mut[T])->(&mut[T],&mut[T]){s.split_at_mut(1)}fn bad(s:&mut[i32]){let(l,_)=split(s);let(_,r)=split(s);l[0]=r[0];}fn main(){}",
                expect: Expect::Invalid("E0499"),
            },
            Case {
                name: "mutable_result_pair_cannot_duplicate_input",
                source: "fn bad<T>(s:&mut[T])->(&mut[T],&mut[T]){(s,s)}fn main(){}",
                expect: Expect::Invalid("E0499"),
            },
        ],
    );
}

#[test]
fn mutable_split_preserves_disjoint_receiver_loans() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "mutable-split",
        &[
            Case {
                name: "generic_partition_and_boundaries",
                source: "fn split<T>(s:&mut[T],n:usize)->(&mut[T],&mut[T]){s.split_at_mut(n)}fn main(){let mut a=[1i32,2,3,4];let(l,r)=split(&mut a,2);l[1]=7;r[0]=9;println!(\"{} {} {} {}\",l[0],l[1],r[0],r[1]);let(l,r)=split(&mut a,0);println!(\"{} {}\",l.len(),r.len());let(l,r)=split(&mut a,4);println!(\"{} {}\",l.len(),r.len());let mut e:[i32;0]=[];let(l,r)=split(&mut e,0);println!(\"{} {}\",l.len(),r.len());}",
                expect: Expect::Run("1 7 9 4\n0 4\n4 0\n0 0\n"),
            },
            Case {
                name: "noncopy_partition_transport",
                source: "struct Item{i:i32}fn split<T>(s:&mut[T],n:usize)->(&mut[T],&mut[T]){s.split_at_mut(n)}fn main(){let mut a=[Item{i:1},Item{i:2},Item{i:3}];let(l,r)=split(&mut a,1);l[0].i=7;r[0].i=9;println!(\"{} {} {} {} {}\",l.len(),r.len(),l[0].i,r[0].i,r[1].i);}",
                expect: Expect::Run("1 2 7 9 3\n"),
            },
            Case {
                name: "receiver_remains_borrowed",
                source: "fn bad(s:&mut[i32]){let(l,r)=s.split_at_mut(1);let again=&mut *s;l[0]=again[0]+r[0];}fn main(){}",
                expect: Expect::Invalid("E0499"),
            },
            Case {
                name: "second_partition_retains_receiver_origin",
                source: "fn bad(s:&mut[i32]){let(_,r)=s.split_at_mut(1);let again=&mut *s;r[0]=again[0];}fn main(){}",
                expect: Expect::Invalid("E0499"),
            },
            Case {
                name: "partition_cannot_be_reborrowed_twice",
                source: "fn bad(s:&mut[i32]){let(l,_)=s.split_at_mut(1);let first=&mut l[0];let second=&mut l[0];*first=*second;}fn main(){}",
                expect: Expect::Invalid("E0499"),
            },
            Case {
                name: "shared_receiver_rejected",
                source: "fn bad(s:&[i32]){let _=s.split_at_mut(0);}fn main(){}",
                expect: Expect::Invalid("E0596"),
            },
            Case {
                name: "local_partition_cannot_escape",
                source: "fn bad<'a>()->(&'a mut[i32],&'a mut[i32]){let mut a=[1,2];a.split_at_mut(1)}fn main(){}",
                expect: Expect::Invalid("E0515"),
            },
        ],
    );
}

#[test]
fn unchanged_sorting_program_matches_stock_at_both_worker_counts() {
    closure_oracle::check(
        "sorting-program",
        &[closure_oracle::Case {
            name: "unchanged_sorting",
            source: include_str!("fixtures/full_programs/16_sorting.rs"),
            expect: closure_oracle::Expect::Run(include_str!(
                "fixtures/full_programs/16_sorting.stdout"
            )),
        }],
    );
}

#[test]
fn inferred_generic_constructor_signatures_check_at_both_worker_counts() {
    for source in [
        "fn shared<'a,T>(p:*const T,n:usize)->&'a[T]{unsafe{std::slice::from_raw_parts(p,n)}}fn main(){}",
        "fn unique<'a,T>(p:*mut T,n:usize)->&'a mut[T]{unsafe{std::slice::from_raw_parts_mut(p,n)}}fn main(){}",
    ] {
        for workers in [1, 8] {
            let output = compiler::compile(source.into(), workers, true).unwrap();
            assert!(output.executable.is_none());
        }
    }
}

#[test]
fn pointer_cast_targets_preserve_nominal_graph_identity() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "raw-cast-targets",
        &[
            Case {
                name: "unchanged_transport",
                source: include_str!("fixtures/raw_slices/transport.rs"),
                expect: Expect::Run(include_str!("fixtures/raw_slices/transport.stdout")),
            },
            Case {
                name: "neighboring_heap_raw_read_write",
                source: include_str!("fixtures/heap/raw_read_write.rs"),
                expect: Expect::Run(include_str!("fixtures/heap/raw_read_write.stdout")),
            },
            Case {
                name: "generic_nominal_and_array_targets",
                source: r#"struct Item<T>{value:T} fn raw<T>(a:&mut[T;2])->*mut T{a as *mut[T;2] as *mut T} fn main(){let mut a=[Item{value:3i32},Item{value:7i32}];let p=raw(&mut a);unsafe{(*p).value=11;}let q=&a as *const[Item<i32>;2] as *const Item<i32>;println!("{} {}",unsafe{(*q).value},a[1].value);}"#,
                expect: Expect::Run("11 7\n"),
            },
        ],
    );
}

#[test]
fn pointer_cast_boundaries_still_reject() {
    use std::io::Write;
    for source in [
        "mod a{pub struct Item{pub value:i32}}mod b{pub struct Item{pub value:i32}}fn main(){let mut a=[a::Item{value:1}];let _=&mut a as *mut[b::Item;1];}",
        "struct Item{value:i32}fn main(){let mut a=[Item{value:1}];let _=&mut a as *mut[Item;2];}",
        "struct Item{value:i32}fn main(){let a=Item{value:1};let _=&a as *mut Item;}",
        "fn bad(a:&mut[i32]){let _=a as *mut i32;}fn main(){}",
    ] {
        let mut stock =
            Command::new(std::env::var_os("JUSTRUST_REAL_RUSTC").unwrap_or_else(|| "rustc".into()))
                .env_remove("RUSTC_BOOTSTRAP")
                .args([
                    "--edition=2024",
                    "--emit=metadata",
                    "--crate-name=cast_boundary",
                    "-o",
                    "/dev/null",
                    "-",
                ])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
        stock
            .stdin
            .take()
            .unwrap()
            .write_all(source.as_bytes())
            .unwrap();
        let output = stock.wait_with_output().unwrap();
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("E0606"));
        for workers in [1, 8] {
            for check in [true, false] {
                let error = match compiler::compile(source.into(), workers, check) {
                    Err(error) => error,
                    Ok(_) => {
                        panic!("invalid pointer cast accepted workers={workers} check={check}")
                    }
                };
                assert!(error.to_string().contains("E0606"), "{error}");
            }
        }
    }
}

fn execute_output(bytes: &[u8]) -> std::process::Output {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::var_os("JCODE_SCRATCH_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let path = root.join(format!("jr-raw-slice-{}-{nonce}", std::process::id()));
    fs::write(&path, bytes).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    let mut child = Command::new(&path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            let _ = child.wait();
            let _ = fs::remove_file(&path);
            panic!("native raw slice timeout");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let output = child.wait_with_output().unwrap();
    fs::remove_file(path).unwrap();
    output
}

fn execute(bytes: &[u8]) -> Vec<u8> {
    let output = execute_output(bytes);
    assert!(output.status.success(), "{output:?}");
    output.stdout
}

#[test]
fn mutable_split_out_of_bounds_fails_before_returning() {
    use std::io::Write;
    for source in [
        "fn main(){let mut a=[1i32,2];let(l,r)=a.split_at_mut(3);println!(\"{} {}\",l.len(),r.len());}",
        "fn main(){let mut a:[i32;0]=[];let(l,r)=a.split_at_mut(1);println!(\"{} {}\",l.len(),r.len());}",
    ] {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::var_os("JCODE_SCRATCH_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let path = root.join(format!("jr-split-bounds-{}-{nonce}", std::process::id()));
        let mut stock =
            Command::new(std::env::var_os("JUSTRUST_REAL_RUSTC").unwrap_or_else(|| "rustc".into()))
                .env_remove("RUSTC_BOOTSTRAP")
                .args([
                    "--edition=2024",
                    "-Cpanic=abort",
                    "--crate-name=split_bounds",
                    "-o",
                ])
                .arg(&path)
                .arg("-")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
        stock
            .stdin
            .take()
            .unwrap()
            .write_all(source.as_bytes())
            .unwrap();
        let compiled = stock.wait_with_output().unwrap();
        assert!(compiled.status.success(), "{compiled:?}");
        let stock_image = fs::read(&path).unwrap();
        fs::remove_file(path).unwrap();
        let stock_output = execute_output(&stock_image);
        assert!(!stock_output.status.success(), "{stock_output:?}");
        assert!(stock_output.stdout.is_empty(), "{stock_output:?}");
        for workers in [1, 8] {
            let checked = compiler::compile(source.into(), workers, true).unwrap();
            assert!(checked.executable.is_none());
            let built = compiler::compile(source.into(), workers, false).unwrap();
            let output = execute_output(&built.executable.expect("BUILD executable"));
            assert!(!output.status.success(), "workers={workers}: {output:?}");
            assert_eq!(output.stdout, stock_output.stdout, "workers={workers}");
        }
    }
}

#[test]
fn shared_mutable_empty_noncopy_and_order_match_stock() {
    let source = include_str!("fixtures/raw_slices/transport.rs");
    let expected = include_bytes!("fixtures/raw_slices/transport.stdout");
    for workers in [1, 8] {
        for check in [true, false] {
            let result = compiler::compile(source.into(), workers, check)
                .unwrap_or_else(|e| panic!("workers={workers},check={check}: {e}"));
            assert_eq!(result.executable.is_none(), check);
            if let Some(bytes) = result.executable {
                assert_eq!(execute(&bytes), expected);
            }
        }
    }
}
