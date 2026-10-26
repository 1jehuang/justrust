use super::*;

fn prelude() -> FileUnit {
    prepare_prelude_unit(
        &SourceOptions::default(),
        &super::super::super::expansion_budget::ExpansionBudget::default(),
    )
    .unwrap()
}

#[test]
fn embedded_issuer_rejects_child_route_and_unit_transport_mutation() {
    let shared = Arc::new(Ok(prelude()));
    let unit = shared.as_ref().as_ref().unwrap();
    let issuer = unit.embedded.as_ref().unwrap();
    issuer.validate(unit).unwrap();
    // `take` is the assembler's normal shallow authority-preserving copy.
    for change in 0..12 {
        let mut changed = SourceAssembler::take(shared.clone());
        match change {
            0 => changed.path.push("spoof"),
            1 => changed.contents = Arc::from(changed.contents.as_ref()),
            2 => changed.text.push(' '),
            3 => changed.origins.reverse(),
            4 => changed.module.push("spoof".into()),
            5 => changed.chain.push(PathBuf::from("spoof")),
            6 => changed.children[0].module.push("spoof".into()),
            7 => changed.children[0].path.push("spoof"),
            8 => changed.attributes.push(PreparedAttribute {
                span: Loader::span(LOCAL_FILE, 0..0),
                name: "spoof".into(),
                effective: true,
            }),
            9 => changed.forbid_unsafe_code = !changed.forbid_unsafe_code,
            10 => changed.edition = Edition::E2021,
            11 => changed.inputs = source_inputs::SourceInputs::untracked(),
            _ => unreachable!(),
        }
        assert!(
            issuer.validate(&changed).is_err(),
            "mutation {change} accepted"
        );
        if !changed.children.is_empty() {
            assert!(
                prepare_child_unit(
                    &changed,
                    0,
                    &SourceOptions::default(),
                    &super::super::super::expansion_budget::ExpansionBudget::default()
                )
                .is_err()
            );
        }
    }
    let mut spoof = prepare_text_unit(
        "<prelude>/lib.rs",
        &unit.contents,
        &SourceOptions::default(),
    )
    .unwrap();
    assert!(spoof.embedded.is_none());
    spoof.embedded = unit.embedded.clone();
    assert!(spoof.embedded.as_ref().unwrap().validate(&spoof).is_err());
}

fn assembled() -> PreparedSource {
    let options = SourceOptions::default();
    let budget = super::super::super::expansion_budget::ExpansionBudget::default();
    let mut assembler = SourceAssembler::new(&options);
    let mut step = assembler
        .provide(Arc::new(prepare_text_unit(
            "issuer.rs",
            "fn main(){}",
            &options,
        )))
        .unwrap();
    loop {
        match step {
            AssemblyStep::Done(source) => return *source,
            AssemblyStep::Need(need) => {
                let unit = match need.parent.as_deref() {
                    None => prepare_prelude_unit(&options, &budget),
                    Some(parent) => prepare_child_unit(parent, need.index, &options, &budget),
                };
                step = assembler.provide(Arc::new(unit)).unwrap();
            }
        }
    }
}

#[test]
fn embedded_issuer_rejects_postassembly_stream_and_attachment_transplants() {
    let source = assembled();
    source
        .embedded_transport
        .as_ref()
        .unwrap()
        .validate(&source)
        .unwrap();
    let mut missing = source.clone();
    missing.embedded_transport = None;
    assert!(crate::compiler::frontend::parse_prepared(&missing).is_err());
    let foreign = assembled();
    for change in 0..9 {
        let mut changed = source.clone();
        match change {
            0 => changed.text.push(' '),
            1 => changed.origins.reverse(),
            2 => {
                changed.sources.files[0].contents =
                    Arc::from(changed.sources.files[0].contents.as_ref())
            }
            3 => changed.embedded_files.reverse(),
            4 => changed.embedded_transport = foreign.embedded_transport.clone(),
            5 => changed.embedded_files = foreign.embedded_files.clone(),
            6 => changed.attributes.push(PreparedAttribute {
                span: Loader::span(LOCAL_FILE, 0..0),
                name: "spoof".into(),
                effective: true,
            }),
            7 => {
                let policy = &mut changed.crate_policies[0];
                policy.forbid_unsafe_code = !policy.forbid_unsafe_code;
            }
            8 => changed.edition = Edition::E2021,
            _ => unreachable!(),
        }
        assert!(
            changed
                .embedded_transport
                .as_ref()
                .unwrap()
                .validate(&changed)
                .is_err(),
            "assembly mutation {change} accepted"
        );
        assert!(
            crate::compiler::frontend::parse_prepared(&changed).is_err(),
            "parser admitted mutation {change}"
        );
    }
}
