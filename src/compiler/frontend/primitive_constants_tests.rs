use super::*;
use crate::compiler::source;

fn genuine_parsed() -> ParsedProgram {
    let options = source::SourceOptions::default();
    let budget = crate::compiler::expansion_budget::ExpansionBudget::default();
    // Obtain the unchanged primitive file through its real embedded parent chain.
    let root = source::prepare_prelude_unit(&options, &budget).unwrap();
    let library = source::prepare_child_unit(&root, 0, &options, &budget).unwrap();
    let index = library
        .children
        .iter()
        .position(|child| child.path.ends_with("primitive_constants.rs"))
        .unwrap();
    let primitive = source::prepare_child_unit(&library, index, &options, &budget).unwrap();
    // Keep the real module route, with only the ordinary Option declarations
    // required by this file. No issuer, original bytes or transport is edited.
    let user_text = r#"
struct Ordinary;
impl Ordinary {}
mod __justrust_std {
    pub mod option { pub enum Option<T> { Some(T), None } }
    pub mod prelude { pub mod v1 {
        pub use super::super::option::Option;
        pub use super::super::option::Option::{Some, None};
    } }
    pub mod primitive_constants;
}
fn main(){let x:i32=i32::MIN;}
"#;
    let fixture_root = std::path::PathBuf::from("/primitive-source-fixture");
    let snapshot = source::SourceSnapshot::new(
        fixture_root.clone(),
        [
            (
                std::path::PathBuf::from("main.rs"),
                user_text.as_bytes().to_vec(),
            ),
            (
                std::path::PathBuf::from("__justrust_std/primitive_constants.rs"),
                primitive.contents.as_bytes().to_vec(),
            ),
        ],
    )
    .unwrap();
    let fixture_options = source::SourceOptions {
        source_snapshot: Some(snapshot),
        ..options
    };
    let user = source::prepare_root_unit(&fixture_root.join("main.rs"), &fixture_options, &budget);
    let mut assembler = source::SourceAssembler::user_only(&fixture_options);
    let source::AssemblyStep::Need(need) = assembler.provide(Arc::new(user)).unwrap() else {
        panic!("primitive source child was not requested");
    };
    // Child module metadata is relative to its declaring file. The minimal
    // fixture inlines the library root instead of loading its separate file.
    let mut mounted_module = root.children[0].module.clone();
    mounted_module.extend(primitive.module.iter().cloned());
    assert_eq!(
        need.parent.as_ref().unwrap().children[need.index].module,
        mounted_module
    );
    let source::AssemblyStep::Done(prepared) = assembler.provide(Arc::new(Ok(primitive))).unwrap()
    else {
        panic!("unexpected primitive source dependency");
    };
    parse_prepared(&prepared).unwrap()
}
fn genuine() -> Program {
    resolve(&genuine_parsed()).unwrap()
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
    let block = &original.impls[index];
    let receipt = block.primitive_source.as_ref().unwrap();
    let budget = crate::compiler::expansion_budget::ExpansionBudget::default();
    let relocated = receipt
        .relocate_parsed(block, index, 17, &original, &budget)
        .unwrap();
    assert_eq!(relocated.implementation, index + 17);
    relocated
        .replay_source(
            block,
            index + 17,
            &original.names,
            &original.embedded_issuers,
        )
        .unwrap();
    assert!(
        receipt
            .relocate_parsed(block, index + 1, 17, &original, &budget)
            .is_err()
    );
    assert!(
        receipt
            .relocate_parsed(block, index, usize::MAX, &original, &budget)
            .is_err()
    );
    let mut forged = block.clone();
    forged.original_source = None;
    assert!(
        receipt
            .relocate_parsed(&forged, index, 17, &original, &budget)
            .is_err()
    );
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

#[test]
fn primitive_method_source_rejects_changed_issuer_owner_signature_and_body() {
    let parsed = genuine_parsed();
    let parsed_index = parsed
        .program
        .impls
        .iter()
        .position(|block| matches!(block.self_ty, Type::Bool) && block.primitive_source.is_some())
        .unwrap();
    let mut changed_parsed = parsed.clone();
    changed_parsed.program.impls[parsed_index].items[0]
        .function
        .body
        .tail = Some(Box::new(Expr::Bool(false)));
    assert!(resolve(&changed_parsed).is_err());
    let mut changed_parsed = parsed.clone();
    changed_parsed.program.impls[parsed_index].items[0]
        .function
        .type_params
        .clear();
    assert!(resolve(&changed_parsed).is_err());
    let original = genuine();
    let index = original
        .impls
        .iter()
        .position(|block| matches!(block.self_ty, Type::Bool) && block.primitive_source.is_some())
        .unwrap();
    let budget = ir::PlanBudget::default();
    let plan = PlanCtx::from_program(&original, &budget).unwrap();
    let receipt = original.impls[index].primitive_source.as_ref().unwrap();
    let declaration = plan.registry.impl_ids[index];
    receipt
        .replay_registration(
            &original,
            &plan.registry,
            index,
            declaration,
            &ir::Ty::Bool,
            &budget,
        )
        .unwrap();
    assert!(
        receipt
            .replay_registration(
                &original,
                &plan.registry,
                index,
                declaration,
                &ir::Ty::Int(ir::IntTy::I32),
                &budget
            )
            .is_err()
    );
    assert!(
        receipt
            .replay_registration(
                &original,
                &plan.registry,
                index,
                plan.registry.impl_ids[index - 1],
                &ir::Ty::Bool,
                &budget
            )
            .is_err()
    );
    for source in [
        "impl bool{fn then_some<T>(self,value:T)->Option<T>{if self{Some(value)}else{None}}}fn main(){}",
        "mod __justrust_std{pub mod primitive_constants{impl bool{fn forged(self)->bool{self}}}}fn main(){}",
    ] {
        assert!(parse(source).unwrap_err().contains("E0390"));
    }
    let mut changed = original.clone();
    changed.impls[index].primitive_source = None;
    rejected(&changed);
    let foreign = genuine();
    let mut changed = original.clone();
    changed.impls[index].primitive_source = foreign.impls[index].primitive_source.clone();
    rejected(&changed);
    let mut changed = original.clone();
    changed.embedded_issuers = foreign.embedded_issuers;
    rejected(&changed);
    let mut changed = original.clone();
    changed.impls[index].original_source = None;
    rejected(&changed);
    let mut changed = original.clone();
    changed.impls[index].self_ty = Type::Int(ir::IntTy::I32);
    rejected(&changed);
    let mut changed = original.clone();
    changed.impls[index].items[0].receiver = ReceiverKind::None;
    rejected(&changed);
    let mut changed = original.clone();
    changed.impls[index].items[0].function.name = original.functions[0].name.clone();
    rejected(&changed);
    let mut changed = original.clone();
    changed.impls[index].items[0].function.type_params.clear();
    rejected(&changed);
    let mut changed = original.clone();
    changed.impls[index].items[0].function.result = Type::Bool;
    rejected(&changed);
    let mut changed = original.clone();
    let Type::Apply(result) = &mut changed.impls[index].items[0].function.result else {
        panic!("generic Option source result missing");
    };
    result.segments.last_mut().unwrap().args[0] = Type::Bool;
    rejected(&changed);
    let mut changed = original.clone();
    changed.impls[index].items[0].function.params[0].1 = Type::Int(ir::IntTy::I32);
    rejected(&changed);
    let mut changed = original.clone();
    changed.impls[index].items[0].function.body.tail = Some(Box::new(Expr::Bool(false)));
    rejected(&changed);
    let mut changed = original.clone();
    changed.impls[index].items[0].function.regions.receiver = Some(RegionName::Static);
    rejected(&changed);
    let mut changed = original.clone();
    changed.impls[index]
        .items
        .push(original.impls[index].items[0].clone());
    rejected(&changed);
}
