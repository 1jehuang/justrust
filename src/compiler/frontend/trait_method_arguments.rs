//! Source-retained UFCS argument domains and canonical publication forgery checks.
use super::*;

#[test]
fn ufcs_method_arguments_remain_separate_and_bounded_source_inputs() {
    let parsed = parse_unresolved("trait Convert<A,B>{fn choose<U,V>(a:A,b:B,u:U,v:V)->U{u}}impl Convert<i32,bool> for i32{}fn probe()->u8{<i32 as Convert<i32,bool>>::choose::<u8,char>(3,true,7u8,'x')}fn main(){}").unwrap();
    let program = &parsed.program;
    let function = &program.functions[0];
    let Expr::TraitCall {
        trait_path,
        type_args,
        ..
    } = function.body.tail.as_deref().unwrap()
    else {
        panic!("missing retained UFCS call")
    };
    let TraitPath::Source(path) = trait_path.as_ref() else {
        panic!("source trait path required")
    };
    assert_eq!(path.segments.last().unwrap().args.len(), 2);
    assert!(matches!(
        type_args.as_slice(),
        [Type::Int(ir::IntTy::U8), Type::Char]
    ));
    let budget = ir::PlanBudget::default();
    program.validate_name_handles().unwrap();
    preflight_program_clone(program, &budget).unwrap();
    recipes::inventory(program, &budget).unwrap();
}

fn canonical_ufcs_publication() -> (ir::PlanBudget, Arc<sr::CanonicalSourcePublication>) {
    use crate::compiler::{CompilerProvider, Input, RESOLVE, SIGNATURES, Snapshot};
    use crate::engine::{Engine, Key};
    let budget = ir::PlanBudget::default();
    let snapshot = Arc::new(Snapshot {
        input: Input::Text("trait Choose{fn choose<U,V>(u:U,v:V)->U;}fn probe<I:Choose>(u:u8,v:char)->u8{<I as Choose>::choose::<u8,char>(u,v)}fn main(){}".into()),
        source_options: crate::compiler::source::SourceOptions::default(),
        budget: budget.clone(),
    });
    let engine = Engine::new(CompilerProvider(snapshot));
    let signatures = engine.request(Key::new(SIGNATURES, vec![]));
    let resolved = engine.request(Key::new(RESOLVE, vec![]));
    engine.run(2).unwrap();
    signatures.completion().unwrap();
    resolved.completion().unwrap();
    let plan = signatures.fact::<PlanCtx>(0).unwrap();
    let program = resolved.fact::<Arc<Program>>(0).unwrap();
    let index = program
        .functions
        .iter()
        .position(|function| {
            function
                .name
                .segments()
                .last()
                .is_some_and(|name| program.names.spelling(*name).unwrap() == "probe")
        })
        .unwrap();
    let declaration = plan.function_decl(index).unwrap();
    let PublishedCallableBody::Source(publication) =
        plan.publish_callable_body(declaration, &budget).unwrap()
    else {
        panic!("canonical source required")
    };
    (budget, publication)
}

#[test]
fn ufcs_ordered_method_actuals_cannot_be_forged_at_source_publication() {
    let (budget, publication) = canonical_ufcs_publication();
    let (_, foreign) = canonical_ufcs_publication();
    let foreign_scope = foreign
        .facts()
        .env()
        .scopes()
        .iter()
        .find(|scope| scope.arity() > 0)
        .unwrap()
        .clone();
    let source = publication.source();
    publication
        .validate_source(source, publication.facts(), &budget)
        .unwrap();
    let call = source
        .exprs
        .iter()
        .position(|expr| {
            matches!(
                &expr.kind,
                ir::ExprKind::Call {
                    target: ir::Callee::TraitItem(_),
                    ..
                }
            )
        })
        .unwrap();
    for mode in 0..4 {
        let mut changed = source.as_ref().clone();
        let ir::ExprKind::Call {
            target: ir::Callee::TraitItem(call),
            ..
        } = &mut changed.exprs[call].kind
        else {
            unreachable!()
        };
        let mut actuals = call.method_args.to_vec();
        assert_eq!(actuals.len(), 2);
        match mode {
            0 => {
                actuals.pop();
            }
            1 => actuals.swap(0, 1),
            2 => actuals[0] = ir::Ty::Bool,
            _ => {
                actuals[0] = ir::Ty::Param(ir::ParamId {
                    owner: foreign_scope.binder(),
                    index: 0,
                })
            }
        }
        call.method_args = actuals.into();
        assert!(
            changed
                .validate_with_facts(ir::FactsCtx::Validated(publication.facts()))
                .is_err(),
            "forged method actuals mode {mode}"
        );
        assert!(
            publication
                .validate_source(&Arc::new(changed), publication.facts(), &budget)
                .is_err()
        );
    }
}
