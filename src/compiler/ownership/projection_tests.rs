//! Opaque ownership shape requires exact world authority.
use super::*;
use crate::compiler::frontend;

fn template() -> cfg::TemplateCfg {
    let program = frontend::parse("trait Has<A,B>{type Out;}fn id<I:Has<i32,bool>>(x:<I as Has<i32,bool>>::Out)-><I as Has<i32,bool>>::Out{x}fn main(){}").unwrap();
    let budget = super::super::ir::PlanBudget::default();
    let plan = frontend::PlanCtx::from_program(&program, &budget).unwrap();
    let declaration = plan.function_decl(0).unwrap();
    let frontend::PublishedCallableBody::Source(publication) =
        plan.publish_callable_body(declaration, &budget).unwrap()
    else {
        panic!("source-owned generic function required")
    };
    cfg::lower_source_template(&publication, &budget).unwrap()
}

#[test]
fn opaque_projection_move_shape_requires_exact_universal_world() {
    let template = template();
    let facts = template.facts();
    let ty = &template.function().locals[template.function().params[0].0].ty;
    let Ty::AssociatedType(projection) = ty else {
        panic!("exact projection input required")
    };
    let context = FactsCtx::from_validated(facts);
    assert!(type_requires_carrier_proof(ty, facts.budget()).unwrap());
    assert!(matches!(
        MoveShape::of(ty, context, &mut Budget::new(10_000)),
        Ok(MoveShape::OpaqueParam)
    ));
    assert!(!context.must_copy(ty).unwrap());
    for raw in [FactsCtx::Concrete, FactsCtx::Universal(facts.env())] {
        assert!(MoveShape::of(ty, raw, &mut Budget::new(10_000)).is_err());
    }
    for mutation in 0..3 {
        let mut forged = projection.as_ref().clone();
        match mutation {
            0 => forged.item.snapshot = super::super::ir::SnapshotId::allocate().unwrap(),
            1 => {
                forged.trait_ref.args = Arc::from([Ty::Bool, Ty::Int(super::super::ir::IntTy::I32)])
            }
            _ => forged.trait_ref.declaration.index = u32::MAX,
        }
        assert!(
            MoveShape::of(
                &Ty::AssociatedType(Arc::new(forged)),
                context,
                &mut Budget::new(10_000)
            )
            .is_err()
        );
    }
    let foreign = self::template();
    assert!(
        MoveShape::of(
            ty,
            FactsCtx::from_validated(foreign.facts()),
            &mut Budget::new(10_000)
        )
        .is_err()
    );
}
