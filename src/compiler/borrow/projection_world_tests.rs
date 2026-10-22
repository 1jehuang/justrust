use super::parameter_equality_tests::source_template;
use super::*;

#[test]
fn projection_family_actual_replays_open_and_closed_authority() {
    let source = "trait Source:Sized{type Out;} struct S; impl Source for S{type Out=i32;} fn id<I:Source>(x:<I as Source>::Out)-><I as Source>::Out{x} fn selected<U>()->i32{id::<S>(9)} fn main(){}";
    for (index, closed) in [(0, false), (1, true)] {
        let template = source_template(source, index);
        let facts = template.facts();
        let view = if closed {
            let target = template
                .function()
                .blocks
                .iter()
                .find_map(|block| match &block.terminator.value {
                    Terminator::Call { target, .. } => Some(target),
                    _ => None,
                })
                .expect("selected has a genuine source call");
            facts
                .carrier_contract(ir::SignatureSubject::Call(target))
                .unwrap()
        } else {
            facts.carrier_contract(ir::SignatureSubject::Body).unwrap()
        };
        let family = view
            .original_contract()
            .inputs
            .iter()
            .find_map(|input| input.location.channel.family())
            .expect("original declaration retains opaque projection input");
        let ir::CarrierFamily::Projection(projection) = &family else {
            panic!("projection must not become a dummy parameter")
        };
        let actual = view.family_actual(&family, facts).unwrap().unwrap();
        let budget = ir::PlanBudget::default();
        if closed {
            assert_eq!(actual, Ty::Int(ir::IntTy::I32));
            assert!(ir::carrier_locations(&actual, &budget).unwrap().is_empty());
        } else {
            assert!(matches!(&actual, Ty::AssociatedType(_)));
            let locations = ir::carrier_locations(&actual, &budget).unwrap();
            assert_eq!(locations.len(), 1);
            assert!(matches!(
                locations[0].channel,
                CarrierChannel::ProjectionTail { .. }
            ));
            assert!(!view.contract().inputs.is_empty());
        }
        for mutation in 0..3 {
            let mut forged = projection.as_ref().clone();
            match mutation {
                0 => forged.item.index = u32::MAX,
                1 => forged.trait_ref.declaration.index = u32::MAX,
                _ => forged.item.snapshot = ir::SnapshotId::standalone().unwrap(),
            }
            assert!(
                view.family_actual(&ir::CarrierFamily::Projection(Arc::new(forged)), facts)
                    .is_err(),
                "foreign projection mutation {mutation}"
            );
        }
        let foreign = source_template(source, index);
        assert!(view.family_actual(&family, foreign.facts()).is_err());
    }
}
