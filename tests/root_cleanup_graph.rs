//! Public compiler graph checks for closed-root cleanup independence.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

use justrust_engine::compiler;
use std::collections::{HashMap, HashSet};

#[test]
fn closed_root_drop_proof_and_object_do_not_await_monomorphization_plan() {
    std::thread::Builder::new().stack_size(32 * 1024 * 1024).spawn(|| {
        let source = "fn f(){let d=D{v:1};} fn unrelated<T>(x:T)->T{x} struct D{v:i32} impl Drop for D{fn drop(&mut self){}} fn main(){f();}";
        for workers in [1, 8] {
            for check_only in [true, false] {
                let result = compiler::compile(source.to_owned(), workers, check_only).unwrap();
                assert_eq!(result.executable.is_none(), check_only);
                assert!(result.analyses.iter().all(|analysis| analysis.passed()));
                let report = result.report;
                assert!(report.capture_complete);
                assert_eq!(report.events_dropped, 0);
                assert_eq!(report.failures, 0);
                let id = |kind, params: &[u64]| {
                    report.node_evaluations.iter().enumerate().find_map(|(position, (key, count))| {
                        if key.kind == kind && key.params == params {
                            assert_eq!(*count, 1);
                            Some(report.node_ids[position])
                        } else { None }
                    }).unwrap()
                };
                assert!(!report.node_evaluations.iter().any(|(key, _)| key.kind == 113),
                    "retired whole-program monomorphization plan returned");
                if check_only {
                    // CHECK analyzes source admission without demanding any
                    // executable instance or cleanup/codegen graph.
                    id(128, &[0]);
                    assert!(!report.node_evaluations.iter().any(|(key, _)|
                        matches!(key.kind, 103 | 105 | 109 | 125 | 142)));
                    continue;
                }
                let instances: HashSet<_> = report.node_evaluations.iter()
                    .zip(&report.node_ids).filter_map(|((key, _), id)|
                        (key.kind == 142).then_some(*id)).collect();
                let demand = id(125, &[0]);
                let proof = id(109, &[0]);
                let tasks: HashMap<_, _> = report.events.iter().filter_map(|event| {
                    Some((event.task?, event.node?))
                }).collect();
                let mut edges: HashMap<_, Vec<_>> = HashMap::new();
                for &(consumer_task, producer, _) in &report.dependency_edges {
                    let consumer = tasks[&consumer_task];
                    edges.entry(consumer).or_default().push(producer);
                }
                for start in std::iter::once(proof)
                    .chain((!check_only).then(|| id(103, &[0]))) {
                    let mut pending = vec![start];
                    let mut seen = HashSet::new();
                    while let Some(node) = pending.pop() {
                        if seen.insert(node) {
                            pending.extend(edges.get(&node).into_iter().flatten().copied());
                        }
                    }
                    assert!(seen.contains(&demand), "cleanup demand not read");
                    assert!(seen.is_disjoint(&instances),
                        "closed-root cleanup awaits generic instance discovery");
                }
                assert_eq!(report.published_kind(103), !check_only);
                assert_eq!(report.published_kind(105), !check_only);
            }
        }
    }).unwrap().join().unwrap();
}
