//! Mutable immediate captures retain independent graph authority.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
use justrust_engine::compiler;

#[test]
fn mutable_captures_keep_independent_checked_bodies_and_immutable_facts() {
    let source = "fn run(mut n:i32,m:i32)->i32{let x=(||->i32{let old=n;n+=m;old+n})();x+(||->i32{n+=1;n})()}fn main(){assert_eq!(run(1,2),8);}";
    let mut previous = None;
    for workers in [1, 8] {
        for check_only in [true, false] {
            let compiled = compiler::compile(source.into(), workers, check_only).unwrap();
            let keys: std::collections::BTreeSet<_> = compiled
                .report
                .node_evaluations
                .iter()
                .filter(|(key, _)| matches!(key.kind, 110 | 112 | 116 | 117))
                .map(|(key, _)| (key.kind, key.params.clone()))
                .collect();
            assert_eq!(keys.iter().filter(|(kind, _)| *kind == 116).count(), 1);
            assert_eq!(keys.iter().filter(|(kind, _)| *kind == 117).count(), 2);
            for kind in [110, 112] {
                assert!(keys.iter().filter(|(actual, _)| *actual == kind).count() >= 4);
            }
            assert!(keys.iter().all(|(_, params)| !params.is_empty()));
            if check_only {
                assert!(
                    compiled
                        .report
                        .node_evaluations
                        .iter()
                        .all(|(key, _)| !matches!(key.kind, 103 | 105))
                );
            }
            if let Some(previous) = &previous {
                assert_eq!(&keys, previous);
            }
            previous = Some(keys);
        }
    }
}
