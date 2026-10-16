use justrust_engine::{Key, compiler};

#[test]
fn compounds_reuse_checked_facts_and_check_has_no_objects() {
    let source = "fn update(p:&mut u64,n:i16){*p+=3u64;*p<<=n;}fn indexed(n:usize)->u64{let mut a=[2u64,10u64];a[n]+=3u64;a[n]/=5u64;a[n]<<=63u8;a[n]}fn main(){let mut x=2u64;update(&mut x,1i16);if x==10u64&&indexed(0usize)==9223372036854775808u64{println!(\"ok\");}}";
    for workers in [1, 8] {
        for check in [true, false] {
            let c = compiler::compile(source.to_owned(), workers, check).unwrap();
            assert!(c.report.capture_complete);
            assert_eq!(c.executable.is_none(), check);
            assert_eq!(c.control_flow.len(), 3);
            assert!(c.analyses.iter().all(|a| a.passed()));
            for index in 0..3 {
                for kind in [102, 108, 109] {
                    let key = Key::new(kind, vec![index]);
                    let evaluations: Vec<_> = c
                        .report
                        .node_evaluations
                        .iter()
                        .filter(|(k, _)| *k == key)
                        .collect();
                    assert_eq!(evaluations.len(), 1);
                    assert_eq!(evaluations[0].1, 1);
                }
            }
            assert_eq!(
                c.report
                    .node_evaluations
                    .iter()
                    .filter(|(k, _)| k.kind == 103)
                    .count(),
                if check { 0 } else { 3 }
            );
        }
    }
}

#[test]
fn unused_compound_bodies_require_valid_endpoints_and_known_counts() {
    for source in [
        "fn bad(){let mut x=1u8;x<<=8u8;}fn main(){}",
        "fn bad(){let mut a=[1u8];let i=0usize;a[i]>>=-1i8;}fn main(){}",
        "fn bad(){let mut x=1u8;x<<=4u8+4u8;}fn main(){}",
        "fn bad(){let mut x=true;x+=false;}fn main(){}",
        "fn bad(){let mut x=1u8;x+=1u16;}fn main(){}",
        "fn bad(p:&u8){*p+=1u8;}fn main(){}",
    ] {
        for workers in [1, 8] {
            for check in [true, false] {
                let failure = compiler::compile_observed(source.to_owned(), workers, check)
                    .err()
                    .unwrap();
                assert!(failure.report.unwrap().capture_complete);
            }
        }
    }
}

#[test]
fn compound_initialized_read_and_loan_failures_block_allbody_publication() {
    for source in [
        "fn bad(){let mut x:u8;x+=1u8;}fn main(){println!(\"must not publish\");}",
        "fn bad(p:&mut u8){let r=&*p;*p+=1u8;let y=*r;}fn main(){println!(\"must not publish\");}",
    ] {
        for workers in [1, 8] {
            for check in [true, false] {
                let failure = compiler::compile_observed(source.to_owned(), workers, check)
                    .err()
                    .unwrap();
                assert!(failure.analyses.iter().any(|(_, a)| !a.passed()));
                let report = failure.report.unwrap();
                assert!(report.capture_complete);
                if check {
                    assert!(!report.node_evaluations.iter().any(|(k, _)| k.kind == 103));
                }
            }
        }
    }
}

#[test]
fn typed_noncompleting_compound_rhs_retains_completion_and_no_constant_value() {
    let source = "fn f()->u8{let mut x=1u8;x<<={return 7u8;0u64};x}fn main(){if f()==7u8{println!(\"ok\");}}";
    for workers in [1, 8] {
        for check in [true, false] {
            let c = compiler::compile(source.to_owned(), workers, check).unwrap();
            assert!(c.report.capture_complete);
            assert_eq!(c.executable.is_none(), check);
            assert!(c.analyses.iter().all(|a| a.passed()));
        }
    }
}
