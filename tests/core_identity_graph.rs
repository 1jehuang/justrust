// Shared helper also exposes declaration preparation for other graph targets.
#[allow(dead_code)]
#[path = "support/native_prelude.rs"]
mod native_prelude;

use justrust_engine::compiler;
use justrust_engine::compiler::source::SourceProbeRequest;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

struct Client(PathBuf);
impl Client {
    fn new(text: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let base = std::env::var_os("JCODE_SCRATCH_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let directory = base.join(format!(
            "jr-identity-graph-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&directory).unwrap();
        let file = directory.join("client.rs");
        std::fs::write(&file, text).unwrap();
        Self(file)
    }
}
impl Drop for Client {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
        let _ = std::fs::remove_dir(self.0.parent().unwrap());
    }
}
fn request() -> SourceProbeRequest {
    SourceProbeRequest {
        lock_path: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("libraries/source-lock.json"),
        item: "core::convert::identity".into(),
    }
}

#[test]
fn original_identity_uses_source_graph_and_ordinary_native_instances() {
    let text = "fn main(){let value=identity::<u128>(340282366920938463463374607431768211455u128);if value==340282366920938463463374607431768211455u128{println!(\"identity graph ok\");}}";
    let client = Client::new(text);
    let mut image = None;
    for workers in [1, 8] {
        for check in [true, false] {
            let compiled =
                compiler::compile_probe_file_observed(client.0.clone(), workers, check, request())
                    .unwrap();
            assert!(compiled.report.capture_complete);
            assert_eq!(compiled.functions, 2);
            assert_eq!(compiled.source_files, 2);
            assert!(compiled.source_bytes > text.len());
            assert_eq!(compiled.executable.is_none(), check);
            let source_nodes: Vec<_> = compiled
                .report
                .node_evaluations
                .iter()
                .filter(|(key, _)| key.kind == 107)
                .collect();
            let library_keys = native_prelude::library_source_keys();
            assert_eq!(source_nodes.len(), library_keys.len() + 1);
            for expected in library_keys {
                assert!(source_nodes.iter().any(|(key, _)| *key == expected));
            }
            assert!(
                source_nodes
                    .iter()
                    .all(|(_, evaluations)| *evaluations == 1)
            );
            if check {
                assert!(
                    compiled
                        .report
                        .node_evaluations
                        .iter()
                        .all(|(key, _)| ![103, 105].contains(&key.kind))
                );
            } else if let Some(previous) = &image {
                assert_eq!(compiled.executable.as_ref().unwrap(), previous);
            } else {
                image = compiled.executable;
            }
        }
    }
}

#[test]
fn source_probe_does_not_skip_unused_client_or_const_body_validation() {
    for text in [
        "fn unused(){unknown_name();}fn main(){}",
        "const fn invalid(x:i32)->i32{unknown_name()}fn main(){}",
        "#[rustc_diagnostic_item=\"convert_identity\"]fn spoof(x:i32)->i32{x}fn main(){}",
    ] {
        let client = Client::new(text);
        for workers in [1, 8] {
            for check in [true, false] {
                let failure = compiler::compile_probe_file_observed(
                    client.0.clone(),
                    workers,
                    check,
                    request(),
                )
                .err()
                .expect("source probe must retain ordinary source validation");
                assert!(failure.report.as_ref().unwrap().capture_complete);
            }
        }
    }
}

#[test]
fn source_probe_requires_exact_registered_item_and_lock() {
    let client = Client::new("fn main(){}");
    for workers in [1, 8] {
        let mut wrong = request();
        wrong.item = "core::convert::other".into();
        let failure = compiler::compile_probe_file_observed(client.0.clone(), workers, true, wrong)
            .err()
            .expect("unknown source item must be rejected");
        assert!(failure.report.as_ref().unwrap().capture_complete);
        assert!(
            failure
                .report
                .as_ref()
                .unwrap()
                .node_evaluations
                .iter()
                .all(|(key, _)| ![103, 105].contains(&key.kind))
        );
    }
}
