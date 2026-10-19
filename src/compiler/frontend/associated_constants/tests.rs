//! Associated member schemas are non-callable and retain exact source ownership.
use super::*;

#[test]
fn initializer_member_authority_replays_source_world_and_dependency_frames() {
    let plan = make_plan(
        "struct Unit;impl Unit{const A:u16=7;const B:u16=Self::A;}fn read()->u16{Unit::B}fn main(){}",
    );
    let member = plan.registry.associated_constants.impls[0][1];
    let input = plan
        .initializer_member_input(member, &[], &plan.budget)
        .unwrap();
    assert_eq!(input.ty(), &ir::Ty::Int(ir::IntTy::U16));
    input.validate(&plan, &plan.budget).unwrap();
    assert!(
        plan.initializer_member_input(member, &[2, 0], &plan.budget)
            .is_err()
    );
    assert!(
        plan.initializer_member_input(member, &[1], &plan.budget)
            .is_err()
    );
    let foreign = make_plan("struct Unit;impl Unit{const A:u16=7;const B:u16=Self::A;}fn main(){}");
    assert!(input.validate(&foreign, &foreign.budget).is_err());
    let declaration = plan.function_decl(0).unwrap();
    let inventory = SourceInventory::build(
        &plan.program,
        plan.callable_body(&plan.program, declaration).unwrap(),
        sr::OriginalBodyId::new(&plan.budget).unwrap(),
        &plan.budget,
    )
    .unwrap();
    let target = ir::CallTarget {
        declaration,
        type_args: Arc::from([]),
    };
    let root = &inventory.associated_constants[0].0.path;
    let mut dependency = root.to_vec();
    dependency.extend([u32::MAX, 0]);
    let selected = plan
        .authorize_associated_constant(&target, &dependency, &plan.budget)
        .unwrap();
    assert_eq!(
        selected.selected().declaration(),
        plan.registry.associated_constants.impls[0][0]
    );
    selected.validate(&plan, &plan.budget).unwrap();
    dependency.push(1);
    assert!(
        plan.authorize_associated_constant(&target, &dependency, &plan.budget)
            .is_err()
    );
    assert!(
        plan.authorize_associated_constant(&target, &[u32::MAX], &plan.budget)
            .is_err()
    );
}

fn make_plan(source: &str) -> PlanCtx {
    PlanCtx::from_program(&parse(source).unwrap(), &ir::PlanBudget::default()).unwrap()
}

#[test]
fn associated_constant_schemas_are_noncallable_and_source_owned() {
    let plan = make_plan(
        "trait Per<T>{type Output; const VALUE:Self::Output;} struct Unit{tag:u8} impl Per<Unit> for Unit{type Output=u8;const VALUE:u8=1;} fn main(){}",
    );
    let world = plan.trait_world().unwrap();
    for declaration in plan
        .registry
        .associated_constants
        .traits
        .iter()
        .chain(plan.registry.associated_constants.impls.iter())
        .flatten()
    {
        let facts = world.associated_constant_facts(*declaration).unwrap();
        assert_eq!(facts.declaration(), *declaration);
        assert!(world.callable_facts(*declaration).is_err());
        assert!(facts.carrier_contract(ir::SignatureSubject::Body).is_err());
        let original = plan.registry.declaration(*declaration).unwrap();
        assert!(source(&plan.registry, original).is_ok());
        let forged = original.clone();
        assert!(source(&plan.registry, &forged).is_err());
    }
    let foreign = make_plan("trait Per<T>{const VALUE:u8=1;} fn main(){}");
    let declaration = foreign.registry.associated_constants.traits[0][0];
    assert!(world.associated_constant_facts(declaration).is_err());
}

#[test]
fn inherent_constant_schema_retains_initializer_without_callable_identity() {
    let plan = make_plan("struct Unit{tag:u8} impl Unit{const VALUE:i32=7;} fn main(){}");
    let declaration = plan.registry.associated_constants.impls[0][0];
    let header = plan
        .associated_constant_schema(declaration, &plan.budget)
        .unwrap();
    assert_eq!(header.ty, ir::Ty::Int(ir::IntTy::I32));
    assert!(header.has_initializer);
    assert!(
        plan.trait_world()
            .unwrap()
            .associated_constant_facts(declaration)
            .is_ok()
    );
}

#[test]
fn closed_constant_member_selection_preserves_override_default_and_world() {
    let plan = make_plan(
        "trait T{const VALUE:u8=3;} impl T for i32{const VALUE:u8=7;} impl T for bool{} fn main(){}",
    );
    let world = plan.trait_world().unwrap();
    let facts = plan
        .instance_facts(&ir::CallTarget {
            declaration: plan.function_decl(0).unwrap(),
            type_args: Arc::from([]),
        })
        .unwrap();
    let member = plan.registry.associated_constants.traits[0][0];
    for (self_ty, expected) in [
        (
            ir::Ty::Int(ir::IntTy::I32),
            plan.registry.associated_constants.impls[0][0],
        ),
        (ir::Ty::Bool, member),
    ] {
        let application = ir::TraitRef {
            declaration: plan.registry.trait_ids[0],
            self_ty,
            args: Arc::from([]),
        };
        let selected = world
            .select_associated_constant(member, &application, &facts)
            .unwrap();
        assert_eq!(selected.member(), member);
        assert_eq!(selected.declaration(), expected);
        assert_eq!(selected.ty(), &ir::Ty::Int(ir::IntTy::U8));
        selected.validate_for(&facts).unwrap();
        let foreign = make_plan("fn main(){}");
        let foreign_facts = foreign
            .instance_facts(&ir::CallTarget {
                declaration: foreign.function_decl(0).unwrap(),
                type_args: Arc::from([]),
            })
            .unwrap();
        assert!(selected.validate_for(&foreign_facts).is_err());
    }
}

#[test]
fn unused_associated_constant_missing_foreign_and_mismatched_members_are_rejected() {
    for (source, diagnostic) in [
        (
            "trait T{const VALUE:u8;} struct Unit{tag:u8} impl T for Unit{} fn main(){}",
            "E0046",
        ),
        (
            "trait T{const VALUE:u8;} struct Unit{tag:u8} impl T for Unit{const VALUE:u16=1;} fn main(){}",
            "E0326",
        ),
        (
            "trait T{const VALUE:u8=1;} struct Unit{tag:u8} impl T for Unit{const OTHER:u8=2;} fn main(){}",
            "E0438",
        ),
    ] {
        let program = parse(source).unwrap();
        let error = PlanCtx::from_program(&program, &ir::PlanBudget::default())
            .err()
            .expect("invalid unused member accepted");
        assert!(error.contains(diagnostic), "{diagnostic}: {error}");
    }
    make_plan("trait T{const VALUE:u8=1;} struct Unit{tag:u8} impl T for Unit{} fn main(){}");
}

#[test]
fn associated_constant_use_authority_replays_original_site_and_closed_arguments() {
    let plan = make_plan(
        "trait T{const VALUE:u8=3;} impl T for i32{const VALUE:u8=7;} impl T for bool{} fn read<X:T>()->u8{<X as T>::VALUE} fn main(){}",
    );
    let declaration = plan.function_decl(0).unwrap();
    let function = plan.callable_body(&plan.program, declaration).unwrap();
    let inventory = SourceInventory::build(
        &plan.program,
        function,
        sr::OriginalBodyId::new(&plan.budget).unwrap(),
        &plan.budget,
    )
    .unwrap();
    assert_eq!(inventory.associated_constants.len(), 1);
    let path = inventory.associated_constants[0].0.path.clone();
    let universal = plan.callable_facts(declaration).unwrap();
    let operand = plan
        .associated_constant_operand(
            declaration,
            &inventory.associated_constants[0].1,
            path.clone(),
            &universal,
            &plan.budget,
        )
        .unwrap();
    assert_eq!(operand.ty(), &ir::Ty::Int(ir::IntTy::U8));
    operand.validate(&universal, &plan.budget).unwrap();
    assert!(
        operand
            .validate(
                &plan.callable_facts(plan.function_decl(1).unwrap()).unwrap(),
                &plan.budget
            )
            .is_err()
    );
    for (ty, expected) in [
        (
            ir::Ty::Int(ir::IntTy::I32),
            plan.registry.associated_constants.impls[0][0],
        ),
        (
            ir::Ty::Bool,
            plan.registry.associated_constants.traits[0][0],
        ),
    ] {
        let caller = ir::CallTarget {
            declaration,
            type_args: Arc::from([ty]),
        };
        let authorization = plan
            .authorize_associated_constant(&caller, &path, &plan.budget)
            .unwrap();
        assert_eq!(authorization.selected().declaration(), expected);
        authorization.validate(&plan, &plan.budget).unwrap();
        assert!(
            plan.authorize_associated_constant(&caller, &[u32::MAX], &plan.budget)
                .is_err()
        );
        let foreign = make_plan("fn main(){}");
        assert!(authorization.validate(&foreign, &foreign.budget).is_err());
    }
    let unrelated = ir::CallTarget {
        declaration: plan.function_decl(1).unwrap(),
        type_args: Arc::from([]),
    };
    assert!(
        plan.authorize_associated_constant(&unrelated, &path, &plan.budget)
            .is_err()
    );
}
