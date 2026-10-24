//! No caller-mutated audit record or foreign source map can issue no_std policy.
use justrust_engine::compiler::{frontend, source};
use std::sync::Arc;

fn prepared(text: &str) -> source::PreparedSource {
    let options = source::SourceOptions {
        library: true,
        ..source::SourceOptions::default()
    };
    let budget = justrust_engine::compiler::token_primitives::ExpansionBudget::default();
    let mut assembler = source::SourceAssembler::new(&options);
    let mut step = assembler
        .provide(Arc::new(source::prepare_text_unit(
            "policy.rs",
            text,
            &options,
        )))
        .unwrap();
    loop {
        match step {
            source::AssemblyStep::Done(prepared) => return *prepared,
            source::AssemblyStep::Need(need) => {
                let unit = match need.parent {
                    None => source::prepare_prelude_unit(&options, &budget),
                    Some(parent) => {
                        source::prepare_child_unit(&parent, need.index, &options, &budget)
                    }
                };
                step = assembler.provide(Arc::new(unit)).unwrap();
            }
        }
    }
}
#[test]
fn public_attribute_mutation_cannot_change_authenticated_policy() {
    let mut no_std = prepared("#![no_std]\npub fn unused() -> Vec<i32> { loop {} }\n");
    let parsed = frontend::parse_prepared(&no_std).unwrap();
    assert!(frontend::resolve(&parsed).is_err());
    no_std.attributes.clear();
    assert!(frontend::parse_prepared(&no_std).is_err());
    let mut ordinary = prepared("pub fn unused() -> Vec<i32> { loop {} }\n");
    frontend::resolve(&frontend::parse_prepared(&ordinary).unwrap()).unwrap();
    ordinary.attributes.push(source::PreparedAttribute {
        span: source::SourceSpan {
            file: source::FileId(0),
            lo: 0,
            hi: 0,
        },
        name: "no_std".into(),
        effective: true,
    });
    assert!(frontend::parse_prepared(&ordinary).is_err());
}
#[test]
fn policy_is_bound_to_original_crate_source_contents() {
    let mut source = prepared("#![no_std]\npub fn value() -> i32 { 3i32 }\n");
    source.sources.files[0].contents = Arc::from("#![no_std]\npub fn value() -> i32 { 9i32 }\n");
    let error = frontend::parse_prepared(&source).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("crate policy source identity mismatch"),
        "{error}"
    );
}
#[test]
fn foreign_snapshot_cannot_launder_issued_crate_policy() {
    let source = prepared("#![no_std]\npub fn unused() -> Vec<i32> { loop {} }\n");
    let mut parsed = frontend::parse_prepared(&source).unwrap();
    let foreign =
        frontend::parse_prepared(&prepared("pub fn unused() -> Vec<i32> { loop {} }\n")).unwrap();
    parsed.program.names = foreign.program.names;
    assert!(frontend::resolve(&parsed).is_err());
}
#[test]
fn configured_no_std_policy_uses_original_effective_cfg() {
    for (cfg, expected) in [("all()", true), ("any()", false)] {
        let source = prepared(&format!(
            "#![cfg_attr({cfg}, no_std)]\npub fn unused() -> Vec<i32> {{ loop {{}} }}\n"
        ));
        let parsed = frontend::parse_prepared(&source).unwrap();
        assert_eq!(frontend::resolve(&parsed).is_err(), expected);
    }
}
