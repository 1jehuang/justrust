use justrust_engine::{Key, compiler};

#[path = "support/closure_oracle.rs"]
mod closure_oracle;

const WRAPPED_REFERENCE: &str = "struct Wrap<T>{value:T}fn id<T>(x:T)->T{x}fn outer<T>(x:T)->T{let wrapped=Wrap{value:x};let out=id::<Wrap<T>>(wrapped);out.value}fn run()->i32{let v=32;let r=outer::<&i32>(&v);*r}\nfn main(){if run()==32{println!(\"generic carriers oracle ok\");}}\n";

// Every source below is byte-identical to the independently frozen carrier oracle.
const ACCEPTED: &[&str] = &[
    "fn put<T>(dst:&mut T,x:T){*dst=x;}fn run()->i32{let a=1;let b=9;let mut slot=&a;put::<&i32>(&mut slot,&b);*slot}\nfn main(){if run()==9{println!(\"generic carriers oracle ok\");}}\n",
    "fn id<T>(x:T)->T{x}fn run()->i32{let mut v=1;let r=&mut v;{let child=id::<&mut i32>(r);*child=14;}*r=15;*r}\nfn main(){if run()==15{println!(\"generic carriers oracle ok\");}}\n",
    "fn walk<T:Copy>(x:T,n:i32)->T{if n==0{x}else{walk::<T>(x,n-1)}}fn run()->i32{let v=16;let r=walk::<&i32>(&v,3);*r}\nfn main(){if run()==16{println!(\"generic carriers oracle ok\");}}\n",
    WRAPPED_REFERENCE,
];
const REJECTED: &[(&str, bool)] = &[
    (
        "fn put<T>(dst:&mut T,x:T){*dst=x;}fn run()->i32{let long=1;let mut slot=&long;{let short=18;put::<&i32>(&mut slot,&short);}*slot}\nfn main(){if run()==0{println!(\"generic carriers oracle ok\");}}\n",
        false,
    ),
    (
        "fn bad<'a,'b>(p:&'a mut &'b mut i32)->&'b mut i32{&mut **p}fn run()->i32{0}\nfn main(){if run()==0{println!(\"generic carriers oracle ok\");}}\n",
        false,
    ),
    (
        "fn bad<'a,T>(x:T)->&'a i32{let local=28;let _=x;&local}fn run()->i32{0}\nfn main(){if run()==0{println!(\"generic carriers oracle ok\");}}\n",
        false,
    ),
];

#[test]
fn wrapped_reference_carrier_matches_stock() {
    closure_oracle::check(
        "wrapped-reference-carrier",
        &[closure_oracle::Case {
            name: "nested_nominal_identity",
            source: WRAPPED_REFERENCE,
            expect: closure_oracle::Expect::Run("generic carriers oracle ok\n"),
        }],
    );
}

fn discovered(report: &justrust_engine::engine::Report, key: Key) -> (u64, u64) {
    let spelling = format!("{key:?}");
    let event = report
        .events
        .iter()
        .find(|event| event.kind == "discovered" && event.detail == spelling)
        .expect("required proof graph node missing");
    (
        event.node.expect("discovery must name its node"),
        event.at_ns,
    )
}

#[test]
fn universal_effect_publication_is_acyclic_and_precedes_every_closed_instance() {
    for source in ACCEPTED {
        for workers in [1, 8] {
            for check_only in [true, false] {
                let result = compiler::compile((*source).to_owned(), workers, check_only).unwrap();
                let report = &result.report;
                assert!(report.capture_complete);
                assert_eq!(report.failures, 0);
                assert_eq!(report.events_dropped, 0);
                assert!(result.analyses.iter().all(|analysis| analysis.passed()));
                assert_eq!(result.executable.is_none(), check_only);
                let admissions: Vec<_> = report
                    .node_evaluations
                    .iter()
                    .filter(|(key, count)| key.kind == 115 && *count == 1)
                    .map(|(key, _)| key.clone())
                    .collect();
                assert!(
                    !admissions.is_empty(),
                    "required universal admission fact missing"
                );
                let publish_of = |key: Key| {
                    let (node, _) = discovered(report, key);
                    report
                        .events
                        .iter()
                        .find(|event| event.kind == "publish" && event.node == Some(node))
                        .unwrap()
                        .at_ns
                };
                let publication = admissions
                    .iter()
                    .map(|key| publish_of(key.clone()))
                    .max()
                    .unwrap();
                for (key, count) in &report.node_evaluations {
                    assert_eq!(*count, 1, "shared node evaluated repeatedly: {key:?}");
                    if key.kind == 115 {
                        let proof = Key::new(112, key.params.clone());
                        let (node, _) = discovered(report, key.clone());
                        let admitted = report
                            .events
                            .iter()
                            .find(|event| event.kind == "publish" && event.node == Some(node))
                            .unwrap()
                            .at_ns;
                        let proved = publish_of(proof);
                        assert!(
                            proved <= admitted,
                            "universal admission preceded its own proof"
                        );
                    }
                    if matches!(key.kind, 110 | 112) {
                        let (node, _) = discovered(report, key.clone());
                        let completed = report
                            .events
                            .iter()
                            .find(|event| event.kind == "publish" && event.node == Some(node))
                            .unwrap()
                            .at_ns;
                        assert!(
                            completed <= publication,
                            "effect table preceded mandatory body proof: {key:?}"
                        );
                    }
                    // Closed analysis reads its callees' universal proofs
                    // directly; the program-wide table gates only CHECK.
                }
                // Every closed analysis that read a universal proof did so
                // through a recorded edge to that proof node.
                let universal: std::collections::HashSet<u64> = report
                    .node_ids
                    .iter()
                    .zip(&report.node_evaluations)
                    .filter(|(_, (key, _))| key.kind == 112)
                    .map(|(id, _)| *id)
                    .collect();
                assert!(
                    universal.is_empty()
                        || report
                            .dependency_edges
                            .iter()
                            .any(|(_, producer, _)| universal.contains(producer)),
                    "universal proofs were never read through graph edges"
                );
                if check_only {
                    assert!(
                        !report
                            .node_evaluations
                            .iter()
                            .any(|(key, _)| key.kind == 103)
                    );
                } else {
                    // Every object follows its own instance proof.
                    let publish_of = |key: Key| {
                        let (node, _) = discovered(report, key);
                        report
                            .events
                            .iter()
                            .find(|event| event.kind == "publish" && event.node == Some(node))
                            .unwrap()
                            .at_ns
                    };
                    for (key, _) in &report.node_evaluations {
                        if key.kind == 103 {
                            let proof = Key::new(109, key.params.clone());
                            assert!(publish_of(key.clone()) >= publish_of(proof));
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn invalid_unused_carriers_and_excluded_inner_actuals_never_emit_objects() {
    for (source, excluded) in REJECTED {
        for workers in [1, 8] {
            for check_only in [true, false] {
                let failure = compiler::compile_observed((*source).to_owned(), workers, check_only)
                    .err()
                    .unwrap_or_else(|| panic!("invalid or excluded carrier program was accepted: workers={workers} check_only={check_only} source={source}"));
                let error = failure.error.to_string().to_lowercase();
                assert!(
                    !["parse error", "unexpected token", "panicked at"]
                        .iter()
                        .any(|substitute| error.contains(substitute)),
                    "carrier graph rejection used a nonsemantic substitute: {error}"
                );
                if *excluded {
                    assert!(error.contains("unsupported"), "{error}");
                } else {
                    assert!(
                        !error.contains("unsupported"),
                        "semantic negative hidden by stage boundary: {error}"
                    );
                }
                let report = failure
                    .report
                    .expect("semantic failure lost graph evidence");
                assert!(report.capture_complete);
                assert!(!report.published_kind(105));
            }
        }
    }
}
