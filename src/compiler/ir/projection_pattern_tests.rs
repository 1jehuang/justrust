use super::*;
use crate::compiler::{cfg, frontend};

fn template() -> cfg::TemplateCfg {
    let source = "trait Has<A,B>{type Out;} fn id<I:Has<i32,bool>>(x:<I as Has<i32,bool>>::Out)-><I as Has<i32,bool>>::Out{x} fn main(){}";
    let program = frontend::parse(source).unwrap();
    let budget = PlanBudget::default();
    let plan = frontend::PlanCtx::from_program(&program, &budget).unwrap();
    let declaration = plan.function_decl(0).unwrap();
    let frontend::PublishedCallableBody::Source(publication) =
        plan.publish_callable_body(declaration, &budget).unwrap()
    else {
        panic!("genuine source body required")
    };
    cfg::lower_source_template(&publication, &budget).unwrap()
}

#[test]
fn opaque_pattern_equality_requires_exact_universal_authority() {
    let template = template();
    let facts = template.facts();
    let ty = &template.function().locals[template.function().params[0].0].ty;
    let Ty::AssociatedType(projection) = ty else {
        panic!("canonical associated input")
    };
    let context = FactsCtx::from_validated(facts);
    assert!(pattern_types_equal_with_facts(ty, ty, &mut 0, context).unwrap());
    assert!(pattern_types_equal(ty, ty, &mut 0).is_err());
    assert!(FactsCtx::Concrete.validate_ty(ty).is_err());
    for mutation in 0..3 {
        let mut forged = projection.as_ref().clone();
        match mutation {
            0 => forged.item.snapshot = SnapshotId::standalone().unwrap(),
            1 => forged.trait_ref.args = Arc::from([Ty::Bool, Ty::Int(IntTy::I32)]),
            _ => forged.trait_ref.declaration.index = u64::MAX,
        }
        let forged = Ty::AssociatedType(Arc::new(forged));
        assert!(
            pattern_types_equal_with_facts(&forged, &forged, &mut 0, context).is_err(),
            "forged identical operands mutation {mutation}"
        );
    }
    let foreign = self::template();
    assert!(
        pattern_types_equal_with_facts(ty, ty, &mut 0, FactsCtx::from_validated(foreign.facts()))
            .is_err()
    );
}
