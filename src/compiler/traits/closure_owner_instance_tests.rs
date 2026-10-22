//! Genuine authored closure templates, never fabricated shape seals.
use super::*;
use crate::compiler::{frontend, source};

struct Fixture {
    plan: frontend::PlanCtx,
    template: Arc<ir::ClosureTy>,
    budget: PlanBudget,
    facts: Arc<ValidatedFacts>,
}

fn fixture() -> Fixture {
    let options = source::SourceOptions::default();
    let expansion = crate::compiler::expansion_budget::ExpansionBudget::default();
    let mut assembler = source::SourceAssembler::new(&options);
    let mut step = assembler.provide(Arc::new(source::prepare_text_unit(
        "closed-owner-reconstruction.rs",
        "trait Marker{}impl Marker for u64{}fn apply<T:Marker>(x:T)->u64{let f=move||{let _y=x;7u64};f()}fn take<U>(u:U)->U{u}fn main(){}",
        &options,
    ))).unwrap();
    let prepared = loop {
        match step {
            source::AssemblyStep::Done(prepared) => break prepared,
            source::AssemblyStep::Need(need) => {
                let unit = match need.parent.as_deref() {
                    None => source::prepare_prelude_unit(&options, &expansion),
                    Some(parent) => {
                        source::prepare_child_unit(parent, need.index, &options, &expansion)
                    }
                };
                step = assembler.provide(Arc::new(unit)).unwrap();
            }
        }
    };
    let program = frontend::resolve(&frontend::parse_prepared(&prepared).unwrap()).unwrap();
    let budget = PlanBudget::default();
    let plan = frontend::PlanCtx::from_program(&program, &budget).unwrap();
    let (provider, owner) = (0..plan.callable_count())
        .find_map(|index| {
            let provider = plan.callable_decl(index).ok()?;
            plan.stored_closure_owner(provider)
                .ok()
                .flatten()
                .map(|(owner, _)| (provider, owner))
        })
        .expect("fixture has a genuine authored generic closure");
    let constants = plan.prepare_body_constants(owner, &budget).unwrap();
    assert!(constants.direct().is_empty());
    let constants = plan
        .seal_body_constants(constants, Vec::new(), &budget)
        .unwrap();
    let (_, closures) = plan
        .publish_callable_body_with_closures(owner, constants, &budget)
        .unwrap();
    let template = closures
        .iter()
        .find(|closure| closure.ty.provider() == provider)
        .unwrap()
        .ty
        .clone();
    let main = plan.callable_decl(plan.closed_roots().unwrap()[0]).unwrap();
    let facts = plan
        .instance_facts(&CallTarget {
            declaration: main,
            type_args: Arc::from([]),
        })
        .unwrap();
    Fixture {
        plan,
        template,
        budget,
        facts,
    }
}

#[test]
fn checked_owner_reconstruction_preserves_seal_and_discharges_real_bounds() {
    let f = fixture();
    let concrete = f
        .facts
        .instantiate_closure_owner(&f.template, &[Ty::Int(ir::IntTy::U64)])
        .unwrap();
    assert_eq!(concrete.owner_args(), &[Ty::Int(ir::IntTy::U64)]);
    assert!(Arc::ptr_eq(
        f.template.shape_seal().unwrap(),
        concrete.shape_seal().unwrap()
    ));
    assert_eq!(concrete.owner(), f.template.owner());
    assert_eq!(concrete.provider(), f.template.provider());
    assert_eq!(concrete.site(), f.template.site());
    let error = f
        .facts
        .instantiate_closure_owner(&f.template, &[Ty::Bool])
        .unwrap_err();
    assert_eq!(
        error.kind,
        FactsErrorKind::Unproved,
        "wrong owner bound: {error:?}"
    );
}

#[test]
fn checked_owner_reconstruction_rejects_bad_arity_phase_and_nonoriginal_template() {
    let f = fixture();
    for arguments in [
        vec![],
        vec![Ty::Int(ir::IntTy::U64), Ty::Bool],
        vec![f.template.owner_args()[0].clone()],
    ] {
        assert!(
            f.facts
                .instantiate_closure_owner(&f.template, &arguments)
                .is_err(),
            "invalid actuals admitted: {arguments:?}"
        );
    }
    let concrete = f
        .facts
        .instantiate_closure_owner(&f.template, &[Ty::Int(ir::IntTy::U64)])
        .unwrap();
    assert!(
        f.facts
            .instantiate_closure_owner(&concrete, &[Ty::Int(ir::IntTy::U64)])
            .is_err()
    );
    let universal = f.plan.callable_facts(f.template.owner()).unwrap();
    assert!(
        universal
            .instantiate_closure_owner(&f.template, &[Ty::Int(ir::IntTy::U64)])
            .is_err()
    );
}

#[test]
fn checked_owner_reconstruction_rejects_foreign_recipe_and_changed_sealed_shape() {
    let f = fixture();
    let foreign = fixture();
    // Independently prepared source worlds allocate distinct snapshots. Keep
    // their authored providers' ordinal correspondence, then explicitly copy
    // the foreign seal onto the local descriptor so numeric identity cannot
    // be the reason the second negative rejects.
    assert_eq!(
        f.template.provider().index,
        foreign.template.provider().index
    );
    assert_ne!(
        f.template.provider().snapshot,
        foreign.template.provider().snapshot
    );
    assert!(
        f.facts
            .instantiate_closure_owner(&foreign.template, &[Ty::Int(ir::IntTy::U64)])
            .is_err()
    );
    let transplanted = Arc::new(
        f.template
            .as_ref()
            .clone()
            .with_shape_seal(foreign.template.shape_seal().cloned()),
    );
    assert_eq!(transplanted.provider(), f.template.provider());
    assert!(
        f.facts
            .instantiate_closure_owner(&transplanted, &[Ty::Int(ir::IntTy::U64)])
            .is_err(),
        "foreign recipe seal admitted with exact local declaration IDs"
    );
    let changed = Arc::new(
        f.template
            .try_map::<String>(|ty| {
                Ok(if ty == &Ty::Int(ir::IntTy::U64) {
                    Ty::Bool
                } else {
                    ty.clone()
                })
            })
            .unwrap(),
    );
    assert!(
        f.facts
            .instantiate_closure_owner(&changed, &[Ty::Int(ir::IntTy::U64)])
            .is_err()
    );
}

#[test]
fn closure_actual_graph_key_roundtrips_through_template_replay() {
    use crate::compiler::instances::graph_key;
    let f = fixture();
    let budget = &f.budget;
    let concrete = f
        .facts
        .instantiate_closure_owner(&f.template, &[Ty::Int(ir::IntTy::U64)])
        .unwrap();
    let take = (0..f.plan.callable_count())
        .map(|i| f.plan.callable_decl(i).unwrap())
        .find(|id| {
            matches!(
                f.plan.persistent_decl_path(*id).unwrap().item(),
                frontend::PersistentDeclItem::Name("take")
            )
        })
        .unwrap();
    let target = CallTarget {
        declaration: take,
        type_args: Arc::from([Ty::Closure(concrete.clone())]),
    };
    let words = graph_key::encode(&target, budget).unwrap();
    let missing = std::cell::RefCell::new(Vec::new());
    let replay = |templates: &graph_key::ClosureTemplates| {
        graph_key::decode_with_closures(&words, &f.plan, budget, &|p, a| {
            graph_key::replay_closure(&f.plan, templates, &missing, budget, p, a)
        })
    };
    // Without the provider template the decoder names exactly what to request.
    assert!(replay(&graph_key::ClosureTemplates::new()).is_err());
    assert_eq!(missing.borrow().as_slice(), &[f.template.provider()]);
    let mut templates = graph_key::ClosureTemplates::new();
    templates.insert(f.template.provider(), f.template.clone());
    assert_eq!(replay(&templates).unwrap(), target);
    // A forged (already substituted) template under the provider is rejected.
    let mut forged = graph_key::ClosureTemplates::new();
    forged.insert(f.template.provider(), concrete);
    assert!(replay(&forged).is_err());
    // The plain decoder still refuses closure actuals without graph replay.
    assert!(graph_key::decode(&words, &f.plan, budget).is_err());
}
