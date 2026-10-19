//! Native boundaries remain owner-local producers in the existing compiler graph.
use justrust_engine::{Key, compiler};

#[test]
fn check_validates_unused_c_boundaries_without_emission_or_linker() {
    let source = "#[repr(C)] struct S{a:u8,b:f64} unsafe extern \"C\"{fn absent(s:S)->S;} #[unsafe(no_mangle)] extern \"C\" fn exported(s:S)->S{s} fn main(){}";
    for workers in [1, 8] {
        let result = compiler::compile(source.to_owned(), workers, true).unwrap();
        assert!(result.report.capture_complete);
        assert!(result.executable.is_none());
        assert!(result.analyses.iter().all(|analysis| analysis.passed()));
        assert!(
            !result
                .report
                .node_evaluations
                .iter()
                .any(|(key, _)| key.kind == 103 || key.kind == 105)
        );
        for index in 0..3 {
            assert_eq!(
                result
                    .report
                    .node_evaluations
                    .iter()
                    .filter(|(key, _)| *key == Key::new(102, vec![index]))
                    .count(),
                1
            );
        }
    }
}

#[test]
fn optimized_object_import_has_no_inlineable_rust_body() {
    use compiler::{source::SourceOptions, ssa::OptMode};
    let source = "unsafe extern \"C\"{fn native_add(a:i32,b:i32)->i32;} fn invoke()->i32{unsafe{native_add(20,22)}} #[unsafe(no_mangle)] extern \"C\" fn rust_add(a:i32,b:i32)->i32{a+b} fn main(){assert_eq!(invoke(),42);assert_eq!(rust_add(19,23),42);}";
    for mode in [OptMode::Adaptive, OptMode::Final] {
        for workers in [1, 8] {
            let mut options = SourceOptions {
                opt: mode,
                ..SourceOptions::default()
            };
            options.native_link.emit_object = true;
            let result =
                compiler::compile_observed_with_options(source.to_owned(), workers, false, options)
                    .unwrap();
            assert!(result.executable.is_some());
            assert_eq!(result.optimizer.inlined, 0);
            assert!(
                result
                    .report
                    .node_evaluations
                    .iter()
                    .any(|(key, _)| *key == Key::new(130, vec![0]))
            );
            assert!(
                result
                    .report
                    .node_evaluations
                    .iter()
                    .any(|(key, _)| *key == Key::new(103, vec![2]))
            );
        }
    }
}

#[test]
fn native_export_is_per_item_and_build_is_check_gated() {
    let source = "#[repr(C)] struct S{a:u8,b:u64} #[unsafe(no_mangle)] extern \"C\" fn exported(s:S)->S{s} fn main(){}";
    for workers in [1, 8] {
        let result = compiler::compile(source.to_owned(), workers, false).unwrap();
        assert!(result.report.capture_complete);
        assert!(result.executable.is_some());
        assert!(result.report.published_kind(104));
        assert!(result.report.published_kind(105));
        for index in 0..2 {
            assert_eq!(
                result
                    .report
                    .node_evaluations
                    .iter()
                    .filter(|(key, _)| *key == Key::new(103, vec![index]))
                    .count(),
                1
            );
        }
        assert!(
            result
                .report
                .node_evaluations
                .iter()
                .filter(|(key, _)| key.params.is_empty())
                .all(|(key, _)| [100, 101, 104, 105, 106, 107, 113, 115].contains(&key.kind))
        );
    }
}
