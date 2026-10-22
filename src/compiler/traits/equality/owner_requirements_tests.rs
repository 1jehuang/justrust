//! Authority controls for exact owner predicate and method binder replay.
use super::*;
use crate::compiler::frontend::{self, PlanCtx};

#[test]
fn predicate_projection_support_does_not_admit_projected_executable_abi() {
    let world =
        source_world("trait Required{type Item;fn run<F>(self,value:Self::Item,f:F);}fn main(){}");
    let items: Vec<_> = world
        .headers
        .items
        .iter()
        .filter(|item| {
            world.catalog.declaration(item.declaration).is_none()
                && world.info(item.declaration).unwrap().owner == Some(item.trait_decl)
                && world.info(item.declaration).unwrap().own_env.arity() == 1
                && world
                    .signature_contains_projection(&item.schema.signature)
                    .unwrap()
        })
        .collect();
    assert_eq!(
        items.len(),
        1,
        "fixture must identify its unique projected generic source method"
    );
    let item = items[0];
    let goal = TraitRef {
        declaration: item.trait_decl,
        self_ty: Ty::Int(ir::IntTy::I32),
        args: Arc::from([]),
    };
    let error = world
        .instantiate_generic_method(
            item,
            &goal,
            &[Ty::Int(ir::IntTy::U8)],
            ProofContext {
                predicates: &world.empty,
                phase: FactsPhase::Closed,
            },
        )
        .unwrap_err();
    assert_eq!(
        error.kind,
        FactsErrorKind::Unsupported,
        "actual ABI rejection: {error:?}"
    );
    assert!(
        format!("{error:?}").contains("generic projected trait method signature"),
        "actual ABI rejection: {error:?}"
    );
}

fn source_world(source: &str) -> Arc<ValidatedTraitWorld> {
    let program = frontend::parse(source).unwrap();
    PlanCtx::from_program(&program, &PlanBudget::default())
        .unwrap()
        .trait_world()
        .unwrap()
}

#[test]
fn required_owner_predicates_preserve_reordered_foreign_application() {
    let world = source_world(
        "trait Bound<A>{}trait Required<A,B>{type Item;fn run<F:Bound<Self::Item>>(self,f:F);}struct Seed;impl Required<i32,u8> for Seed{type Item=i32;fn run<F:Bound<i32>>(self,f:F){}}impl Required<u8,i32> for Seed{type Item=u8;fn run<F:Bound<u8>>(self,f:F){}}fn main(){}",
    );
    let owners: Vec<_> = world
        .headers
        .impls
        .iter()
        .filter(|owner| !owner.associated_types.is_empty())
        .collect();
    assert_eq!(owners.len(), 2);
    let first = owners[0];
    let target = world.callable_header(first.items[0].1).unwrap();
    let item = world.item_header(first.items[0].0).unwrap();
    let raw = world
        .alpha_rename_method_schema(
            item,
            &first.trait_ref,
            &world.method_arguments(target.declaration).unwrap(),
            &target.schema.env,
        )
        .unwrap();
    let normalized = world
        .normalize_required_owner_predicates(target.declaration, &raw.predicates)
        .unwrap();
    assert_eq!(raw.predicates.len(), normalized.len());
    let foreign = world
        .normalize_required_owner_predicates(owners[1].items[0].1, &raw.predicates)
        .unwrap();
    world
        .compare_predicate_rows(&raw.predicates, &foreign)
        .unwrap();
    assert!(world.compare_predicate_rows(&normalized, &foreign).is_err());
    for (raw, actual) in raw.predicates.iter().zip(normalized.iter()) {
        assert_eq!(raw.trait_ref().declaration, actual.trait_ref().declaration);
    }
}

#[test]
fn required_owner_predicates_do_not_authorize_stronger_method_bound() {
    let source = frontend::parse("trait Bound<A>{}trait Required{type Item;fn run<F:Bound<Self::Item>>(self,f:F);}struct Seed;impl Required for Seed{type Item=i32;fn run<F:Bound<u8>>(self,f:F){}}fn main(){}").unwrap();
    let result = PlanCtx::from_program(&source, &PlanBudget::default());
    assert!(matches!(result, Err(error) if error.contains("strengthens required predicates")));
}

#[test]
fn cyclic_assignment_is_rejected_at_closed_projection_boundary() {
    let source = frontend::parse("trait Bound<A>{}trait Required{type Item;fn run<F:Bound<Self::Item>>(self,f:F);}struct Seed;impl Required for Seed{type Item=Self::Item;fn run<F:Bound<Self::Item>>(self,f:F){}}fn main(){}").unwrap();
    let result = PlanCtx::from_program(&source, &PlanBudget::default());
    // The rigid closed-projection guard rejects this cycle before a cycle-specific
    // normalization proof can be produced. Preserve that precise boundary.
    match result {
        Err(error) => assert_eq!(
            error,
            "closed associated projection must normalize before execution"
        ),
        Ok(_) => panic!("cyclic associated assignment was admitted"),
    }
}

#[test]
fn owner_callable_replay_distinguishes_implementation_and_method_actuals() {
    let world = source_world(
        "struct Wrap<T>{value:T}trait Bound<A>{}impl Bound<u8> for u32{}trait Required{type Item;fn run<F:Bound<Self::Item>>(self,f:F)->i32;}impl<T> Required for Wrap<T>{type Item=T;fn run<F:Bound<T>>(self,f:F)->i32{7}}fn main(){}",
    );
    let owner = world
        .headers
        .impls
        .iter()
        .find(|owner| !owner.associated_types.is_empty())
        .unwrap();
    let declaration = owner.items[0].1;
    let u8_ty = Ty::Int(ir::IntTy::U8);
    let u32_ty = Ty::Int(ir::IntTy::U32);
    world
        .instance_facts(&CallTarget {
            declaration,
            type_args: Arc::from([u8_ty.clone(), u32_ty.clone()]),
        })
        .unwrap();
    for arguments in [
        Arc::<[Ty]>::from([u8_ty.clone()]),
        Arc::<[Ty]>::from([u32_ty.clone(), u8_ty.clone()]),
        Arc::<[Ty]>::from([u8_ty.clone(), u32_ty.clone(), u8_ty.clone()]),
    ] {
        assert!(
            world
                .instance_facts(&CallTarget {
                    declaration,
                    type_args: arguments
                })
                .is_err()
        );
    }
    let raw = world.callable_header(declaration).unwrap();
    let foreign_method_parameter = raw.schema.env.ordered_params().last().unwrap();
    assert!(
        world
            .instance_facts(&CallTarget {
                declaration,
                type_args: Arc::from([u8_ty, Ty::Param(foreign_method_parameter)])
            })
            .is_err()
    );
}

/// Source fixture: a generic nominal whose canonical impl assigns `Item`, and
/// a same-named trait from a different declaration.
fn rigid_nominal_world() -> (
    Arc<ValidatedTraitWorld>,
    Arc<ValidatedFacts>,
    TraitRef,
    DeclId,
) {
    let world = source_world(
        "mod other{pub trait Next{type Item;}}trait Next{type Item;}struct Cursor<T>{v:T}\
         impl<T> Next for Cursor<T>{type Item=T;}fn run<T>(x:Cursor<T>){}fn main(){}",
    );
    let callable = world
        .headers
        .callables
        .iter()
        .find(|c| {
            world.catalog.declaration(c.declaration).is_none()
                && c.schema.env.total_arity() == 1
                && matches!(c.schema.signature.params.first(), Some(Ty::Struct(_)))
        })
        .unwrap()
        .declaration;
    let facts = world.callable_facts(callable).unwrap();
    let cursor = world
        .callable_header(callable)
        .unwrap()
        .schema
        .signature
        .params[0]
        .clone();
    let owner = world
        .headers
        .impls
        .iter()
        .find(|h| !h.associated_types.is_empty())
        .unwrap();
    let goal = TraitRef {
        declaration: owner.trait_ref.declaration,
        self_ty: cursor,
        args: Arc::from([]),
    };
    (
        world.clone(),
        facts,
        goal,
        owner.associated_types[0].associated,
    )
}

#[test]
fn rigid_nominal_equality_uses_canonical_impl_and_rejects_same_named_or_wrong_rhs() {
    let (world, facts, goal, item) = rigid_nominal_world();
    let context = ProofContext::from_facts(&facts);
    let projection = ir::AssociatedTypeProjection {
        trait_ref: goal.clone(),
        item,
    };
    let Ty::Struct(cursor) = &goal.self_ty else {
        unreachable!()
    };
    let element = cursor.def().type_args[0].clone();
    assert!(matches!(
        world
            .prove_projection_equality(&projection, &element, context, &mut ProofScratch::default())
            .unwrap(),
        ProofOutcome::Proved(_)
    ));
    assert!(matches!(
        world
            .prove_projection_equality(
                &projection,
                &Ty::Bool,
                context,
                &mut ProofScratch::default()
            )
            .unwrap(),
        ProofOutcome::NotProven
    ));
    // Same-named trait with a different declaration has no implementation.
    let other = world
        .headers
        .traits
        .iter()
        .find(|t| {
            t.declaration != goal.declaration
                && world.catalog.declaration(t.declaration).is_none()
                && t.associated_types.len() == 1
        })
        .unwrap();
    let foreign = ir::AssociatedTypeProjection {
        trait_ref: TraitRef {
            declaration: other.declaration,
            ..goal.clone()
        },
        item: other.associated_types[0],
    };
    assert!(
        !matches!(
            world.prove_projection_equality(
                &foreign,
                &element,
                context,
                &mut ProofScratch::default()
            ),
            Ok(ProofOutcome::Proved(_))
        ),
        "same-named trait must not reuse another declaration's assignment"
    );
    // Same item paired with the foreign trait identity is rejected as metadata.
    let mixed = ir::AssociatedTypeProjection {
        trait_ref: foreign.trait_ref.clone(),
        item,
    };
    assert!(
        world
            .prove_projection_equality(&mixed, &element, context, &mut ProofScratch::default())
            .is_err()
    );
}

#[test]
fn implementation_equality_replay_rejects_forged_and_foreign_membership() {
    let (world, facts, goal, item) = rigid_nominal_world();
    let context = ProofContext::from_facts(&facts);
    let projection = ir::AssociatedTypeProjection {
        trait_ref: goal.clone(),
        item,
    };
    let (membership, normal) = world
        .rigid_nominal_implementation_rhs(&projection, context, &mut ProofScratch::default())
        .unwrap()
        .unwrap();
    world
        .implementation_equality_rhs(&projection, &membership, context)
        .unwrap();
    let forge = |edit: &dyn Fn(&mut ProofRecord)| {
        let mut record = (*membership.0).clone();
        edit(&mut record);
        ProofEvidence(Arc::new(record))
    };
    let forged = [
        forge(&|r| r.world = WorldId(SnapshotId::standalone().unwrap())),
        forge(&|r| r.environment = usize::MAX),
        forge(&|r| r.phase = FactsPhase::Closed),
        forge(&|r| r.origin = ProofOrigin::Assumption),
        forge(&|r| {
            if let ProofOrigin::Implementation { arguments, .. } = &mut r.origin {
                *arguments = Arc::from([]);
            }
        }),
        forge(&|r| {
            if let ProofOrigin::Implementation { declaration, .. } = &mut r.origin {
                declaration.index = u64::MAX;
            }
        }),
    ];
    for (index, evidence) in forged.iter().enumerate() {
        assert!(
            world
                .implementation_equality_rhs(&projection, evidence, context)
                .is_err(),
            "forged membership {index} replayed"
        );
        let goal = Predicate::ProjectionEq {
            projection: projection.clone(),
            ty: normal.clone(),
        };
        assert!(
            world
                .evidence(
                    &goal,
                    context,
                    ProofOrigin::ImplementationEquality {
                        membership: evidence.clone()
                    }
                )
                .is_err(),
            "forged equality origin {index} admitted"
        );
    }
    // A foreign world cannot replay this world's membership either.
    let (foreign, foreign_facts, _, _) = rigid_nominal_world();
    assert!(
        foreign
            .implementation_equality_rhs(
                &projection,
                &membership,
                ProofContext::from_facts(&foreign_facts)
            )
            .is_err()
    );
}

#[test]
fn owner_assignment_retains_rigid_param_projection_only_with_owner_membership() {
    // Bound present: `I::Item` stays symbolic under the owner's own predicate.
    source_world(
        "trait Next{type Item;}struct Wrap<I>{i:I}\
         impl<I:Next> Next for Wrap<I>{type Item=(usize,I::Item);}fn main(){}",
    );
    // Bound missing from the owner predicates: still rejected.
    let source = frontend::parse(
        "trait Next{type Item;}struct Wrap<I>{i:I}\
         impl<I> Next for Wrap<I>{type Item=(usize,I::Item);}fn main(){}",
    );
    let admitted = source
        .ok()
        .and_then(|program| PlanCtx::from_program(&program, &PlanBudget::default()).ok());
    assert!(
        admitted.is_none(),
        "rigid projection without owner membership was admitted"
    );
}
