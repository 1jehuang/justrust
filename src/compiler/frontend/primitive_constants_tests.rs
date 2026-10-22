use super::*;
use crate::compiler::source;

fn genuine() -> Program {
    let options = source::SourceOptions::default();
    let budget = crate::compiler::expansion_budget::ExpansionBudget::default();
    let mut assembler = source::SourceAssembler::new(&options);
    let mut step = assembler
        .provide(Arc::new(source::prepare_text_unit(
            "primitive-min.rs",
            "fn main(){let x:i32=i32::MIN;}",
            &options,
        )))
        .unwrap();
    let prepared = loop {
        match step {
            source::AssemblyStep::Done(prepared) => break prepared,
            source::AssemblyStep::Need(need) => {
                let unit = match need.parent.as_deref() {
                    None => source::prepare_prelude_unit(&options, &budget),
                    Some(parent) => {
                        source::prepare_child_unit(parent, need.index, &options, &budget)
                    }
                };
                step = assembler.provide(Arc::new(unit)).unwrap();
            }
        }
    };
    resolve(&parse_prepared(&prepared).unwrap()).unwrap()
}
fn rejected(program: &Program) {
    assert!(PlanCtx::from_program(program, &ir::PlanBudget::default()).is_err());
}

#[test]
fn primitive_min_check_uses_canonical_declaration_without_codegen() {
    use crate::compiler::{
        ASSOCIATED_CONST_HEADER, BUILD, CompilerProvider, INSTANCE, Input, OBJECT, Snapshot,
    };
    use crate::engine::Engine;
    for workers in [1, 8] {
        let engine = Engine::new(CompilerProvider(Arc::new(Snapshot {
            input: Input::Text("fn main(){let value:i32=i32::MIN;}".into()),
            source_options: source::SourceOptions::default(),
            budget: ir::PlanBudget::default(),
        })));
        let root = engine.request(crate::compiler::keys::CheckRoot.key());
        let report = engine.run(workers).unwrap();
        root.completion().unwrap();
        assert!(
            report
                .node_evaluations
                .iter()
                .any(|(key, _)| key.kind == ASSOCIATED_CONST_HEADER)
        );
        assert!(
            !report
                .node_evaluations
                .iter()
                .any(|(key, _)| matches!(key.kind, BUILD | INSTANCE | OBJECT)),
            "primitive MIN CHECK demanded executable work"
        );
    }
}

#[test]
fn primitive_min_source_issuer_rejects_raw_and_transplanted_authority() {
    let original = genuine();
    let index = original
        .impls
        .iter()
        .position(|block| block.primitive_source.is_some())
        .unwrap();
    PlanCtx::from_program(&original, &ir::PlanBudget::default()).unwrap();
    for source in [
        "impl i32{pub const MIN:i32=-2147483647i32-1i32;}fn main(){}",
        "mod __justrust_std{pub mod primitive_constants{impl i32{pub const MIN:i32=-2147483647i32-1i32;}}}fn main(){}",
    ] {
        let error = parse(source).expect_err("user primitive impl accepted");
        assert!(error.contains("E0390"), "{error}");
    }
    let mut missing = original.clone();
    missing.impls[index].primitive_source = None;
    rejected(&missing);
    let mut missing_issuer = original.clone();
    missing_issuer.embedded_issuers = Arc::from([]);
    rejected(&missing_issuer);
    let foreign = genuine();
    let mut changed = original.clone();
    changed.impls[index].primitive_source = foreign
        .impls
        .iter()
        .find_map(|block| block.primitive_source.clone());
    rejected(&changed);
    let mut changed = original.clone();
    changed.embedded_issuers = foreign.embedded_issuers;
    rejected(&changed);
    let mut changed = original.clone();
    changed.impls[index].original_source = foreign.impls[index].original_source;
    rejected(&changed);
    let mut changed = original.clone();
    changed.impls[index].module = original.functions[0].module;
    rejected(&changed);
    let mut changed = original.clone();
    changed.impls[index].original_source = None;
    rejected(&changed);
    let mut changed = original.clone();
    changed.impls[index].self_ty = Type::Int(ir::IntTy::I64);
    rejected(&changed);
    let mut changed = original.clone();
    changed.impls[index].associated_constants[0].ty = Type::Int(ir::IntTy::I64);
    rejected(&changed);
    let mut changed = original.clone();
    changed.impls[index].associated_constants[0].initializer =
        Some(Expr::Integer(IntegerLiteral {
            magnitude: 0,
            suffix: Some(ir::IntTy::I32),
        }));
    rejected(&changed);
    let mut changed = original.clone();
    changed.impls.push(original.impls[index].clone());
    rejected(&changed);
}
