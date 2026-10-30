//! Canonical source-provider mapping for an opaque producer's layout contract.
//!
//! This is not an executable call proof. In particular it does not fabricate
//! actual arguments for a generic method to force executable trait selection.
//! It retains the membership proof and maps only the implementation prefix,
//! leaving the method's canonical own binder as separate conditional inputs.
use super::*;

/// Canonical public method applications of one authored opaque packing site.
/// A forwarded opaque value stays a forwarded contract, not a guessed concrete
/// provider. The producer consumer must discharge that source contract before
/// admitting any invocation.
#[derive(Clone)]
pub(in crate::compiler) enum BoundProvider {
    Concrete {
        public: TraitRef,
        seed: ProviderDomainSeed,
    },
    Forwarded {
        public: TraitRef,
        application: TraitRef,
        item: DeclId,
    },
}

impl BoundProvider {
    pub(in crate::compiler) fn validate_same(
        &self,
        expected: &Self,
        facts: &ValidatedFacts,
    ) -> Result<()> {
        let world = facts.world();
        let (application, item, original, original_item) = match (self, expected) {
            (
                Self::Concrete { public, seed: a },
                Self::Concrete {
                    public: original,
                    seed: b,
                },
            ) => {
                if !world.eq_predicate(
                    &Predicate::Implements(public.clone()),
                    &Predicate::Implements(original.clone()),
                )? {
                    return Err(FactsError::invalid(
                        facts.budget(),
                        "opaque contract changed public applied method",
                    ));
                }
                a.validate()?;
                b.validate()?;
                if a.declaration != b.declaration
                    || a.method_arity != b.method_arity
                    || !world.eq_types(&a.prefix, &b.prefix, 0)?
                {
                    return Err(FactsError::invalid(
                        facts.budget(),
                        "opaque contract changed source provider or own binder",
                    ));
                }
                (&a.application, a.item, &b.application, b.item)
            }
            (
                Self::Forwarded {
                    public,
                    application,
                    item,
                },
                Self::Forwarded {
                    public: original_public,
                    application: original,
                    item: original_item,
                },
            ) => {
                if !world.eq_predicate(
                    &Predicate::Implements(public.clone()),
                    &Predicate::Implements(original_public.clone()),
                )? {
                    return Err(FactsError::invalid(
                        facts.budget(),
                        "forwarded contract changed public applied method",
                    ));
                }
                (application, *item, original, *original_item)
            }
            _ => {
                return Err(FactsError::invalid(
                    facts.budget(),
                    "opaque contract changed concrete/forwarded source evidence",
                ));
            }
        };
        if item != original_item
            || !world.eq_predicate(
                &Predicate::Implements(application.clone()),
                &Predicate::Implements(original.clone()),
            )?
        {
            return Err(FactsError::invalid(
                facts.budget(),
                "opaque contract changed original applied bound method",
            ));
        }
        Ok(())
    }
}

pub(in crate::compiler) fn bound_providers(
    facts: Arc<ValidatedFacts>,
    opaque: &Arc<ir::OpaqueTy>,
    authored_value: &Ty,
) -> Result<Vec<BoundProvider>> {
    let world = facts.world();
    let budget = facts.budget();
    world.check_facts(&facts, true)?;
    if facts.phase() != FactsPhase::Closed || facts.declaration() != opaque.owner() {
        return Err(FactsError::invalid(
            budget,
            "opaque alignment contract changed original packing owner",
        ));
    }
    facts.validate_ty_metadata(&Ty::Opaque(opaque.clone()))?;
    facts.require_sized(authored_value)?;
    let mut pending = Vec::new();
    for bound in opaque.bounds() {
        let application = bound.application(authored_value.clone());
        for (item, ty) in bound.equalities() {
            match world.prove(
                &Predicate::ProjectionEq {
                    projection: ir::AssociatedTypeProjection {
                        trait_ref: application.clone(),
                        item: *item,
                    },
                    ty: ty.clone(),
                },
                &facts,
                &mut ProofScratch::default(),
            )? {
                ProofOutcome::Proved(_) => {}
                ProofOutcome::NotProven => {
                    return Err(FactsError::unproved(
                        budget,
                        "opaque alignment producer omitted original associated equality",
                    ));
                }
            }
        }
        let public = bound.application(Ty::Opaque(opaque.clone()));
        push(budget, &mut pending, (public, application))?;
    }
    let mut seen: Vec<TraitRef> = Vec::new();
    let mut result = Vec::new();
    while let Some((public, application)) = pending.pop() {
        charged(budget, budget.charge_work(1))?;
        let mut duplicate = false;
        for previous in &seen {
            if world.eq_predicate(
                &Predicate::Implements(previous.clone()),
                &Predicate::Implements(public.clone()),
            )? {
                duplicate = true;
                break;
            }
        }
        if duplicate {
            continue;
        }
        match world.prove(
            &Predicate::Implements(application.clone()),
            &facts,
            &mut ProofScratch::default(),
        )? {
            ProofOutcome::Proved(_) => {}
            ProofOutcome::NotProven => {
                return Err(FactsError::unproved(
                    budget,
                    "opaque alignment producer omitted original bound membership",
                ));
            }
        }
        // Substitute actual trait arguments, rather than traversing only the
        // declaration IDs as the private vtable's slot-order helper does.
        let authored_supers = world.supertrait_predicates_for(&application, facts.env())?;
        let public_supers = world.supertrait_predicates_for(&public, facts.env())?;
        if authored_supers.len() != public_supers.len() {
            return Err(FactsError::invalid(
                budget,
                "opaque supertrait application inventory changed shape",
            ));
        }
        for (authored, exposed) in authored_supers.into_iter().zip(public_supers) {
            match (authored, exposed) {
                (Predicate::Implements(parent), Predicate::Implements(public_parent)) => {
                    push(budget, &mut pending, (public_parent, parent))?
                }
                _ => {
                    return Err(FactsError::invalid(
                        budget,
                        "opaque supertrait method inventory changed predicate family",
                    ));
                }
            }
        }
        for &item in world.trait_header(application.declaration)?.items.iter() {
            let provider = if matches!(authored_value, Ty::Opaque(_)) {
                BoundProvider::Forwarded {
                    public: public.clone(),
                    application: application.clone(),
                    item,
                }
            } else {
                BoundProvider::Concrete {
                    public: public.clone(),
                    seed: capture(facts.clone(), &application, item)?,
                }
            };
            push(budget, &mut result, provider)?;
        }
        push(budget, &mut seen, public)?;
    }
    Ok(result)
}

#[derive(Clone)]
pub(in crate::compiler) struct ProviderDomainSeed {
    facts: Arc<ValidatedFacts>,
    application: TraitRef,
    item: DeclId,
    membership: ProofEvidence,
    declaration: DeclId,
    prefix: Arc<[Ty]>,
    method_arity: usize,
}
impl ProviderDomainSeed {
    pub(in crate::compiler) fn item(&self) -> DeclId {
        self.item
    }
    pub(in crate::compiler) fn declaration(&self) -> DeclId {
        self.declaration
    }
    pub(in crate::compiler) fn prefix(&self) -> &[Ty] {
        &self.prefix
    }
    pub(in crate::compiler) fn method_arity(&self) -> usize {
        self.method_arity
    }
    pub(in crate::compiler) fn validate(&self) -> Result<()> {
        let replay = capture(self.facts.clone(), &self.application, self.item)?;
        let budget = self.facts.budget();
        if self.declaration != replay.declaration
            || self.method_arity != replay.method_arity
            || !self.facts.world.eq_types(&self.prefix, &replay.prefix, 0)?
            || self.membership.0.world != replay.membership.0.world
            || self.membership.0.environment != replay.membership.0.environment
            || self.membership.0.phase != replay.membership.0.phase
            || !self
                .facts
                .world
                .eq_predicate(&self.membership.0.goal, &replay.membership.0.goal)?
        {
            return Err(FactsError::invalid(
                budget,
                "source provider domain contract changed canonical membership or prefix",
            ));
        }
        Ok(())
    }
}

pub(in crate::compiler) fn capture(
    facts: Arc<ValidatedFacts>,
    application: &TraitRef,
    item: DeclId,
) -> Result<ProviderDomainSeed> {
    let world = facts.world();
    let budget = facts.budget();
    world.check_facts(&facts, true)?;
    if facts.phase() != FactsPhase::Closed {
        return Err(FactsError::invalid(
            budget,
            "opaque provider domain requires exact closed producer facts",
        ));
    }
    let header = world.item_header(item)?;
    if header.trait_decl != application.declaration {
        return Err(FactsError::invalid(
            budget,
            "opaque provider domain changed canonical bound method owner",
        ));
    }
    let parent_arity = world.trait_header(header.trait_decl)?.env.total_arity();
    let method_arity = header
        .schema
        .env
        .total_arity()
        .checked_sub(parent_arity)
        .ok_or_else(|| {
            FactsError::invalid(budget, "opaque bound method lost canonical generic parent")
        })?;
    let membership = match world.prove(
        &Predicate::Implements(application.clone()),
        &facts,
        &mut ProofScratch::default(),
    )? {
        ProofOutcome::Proved(evidence) => evidence,
        ProofOutcome::NotProven => {
            return Err(FactsError::unproved(
                budget,
                "opaque producer omitted required canonical bound membership",
            ));
        }
    };
    let mut origin = &membership;
    while let ProofOrigin::ProjectedSubject { evidence, .. } = &origin.0.origin {
        charged(budget, budget.charge_work(1))?;
        origin = evidence;
    }
    let (declaration, prefix) = if let ProofOrigin::Implementation {
        declaration,
        arguments,
    } = &origin.0.origin
    {
        let positions = world.impl_header_index.get(budget, *declaration)?;
        let mut mapping = None;
        for &position in positions.iter() {
            charged(budget, budget.charge_work(1))?;
            let implementation = &world.headers.impls[position];
            if implementation.declaration != *declaration {
                continue;
            }
            for (required, body) in implementation.items.iter() {
                charged(budget, budget.charge_work(1))?;
                if *required == item {
                    mapping = Some((*body, arguments.clone()));
                    break;
                }
            }
            if mapping.is_none()
                && let Some(default) = header.default
            {
                world.ensure_provider(default)?;
                mapping = Some((default, world.trait_arguments(application)?.into()));
            }
            break;
        }
        mapping.ok_or_else(|| {
            FactsError::invalid(
                budget,
                "opaque selected implementation omitted required source provider",
            )
        })?
    } else if let ProofOrigin::Closure { kind } = &origin.0.origin {
        // Source-domain mapping needs the selected authored provider, not an
        // executable projected signature. An opaque capture does not change
        // the canonical closure Fn head or its membership proof.
        let Ty::Closure(closure) = &application.self_ty else {
            return Err(FactsError::invalid(
                budget,
                "closure source proof lost exact Self",
            ));
        };
        if method_arity != 0 || world.closure_trait_kind(application.declaration)? != Some(*kind) {
            return Err(FactsError::invalid(
                budget,
                "closure source proof changed callable trait",
            ));
        }
        let mut scratch = ProofScratch::default();
        for (ty, usage) in std::iter::once((&application.self_ty, TypeUse::Root))
            .chain(application.args.iter().map(|ty| (ty, TypeUse::Actual)))
        {
            if world
                .closedness_of_root(ty, facts.env(), usage, 0, &mut scratch)?
                .has_parameters
            {
                return Err(FactsError::unsupported(
                    budget,
                    "unsupported: rigid closure source provider",
                ));
            }
        }
        let target = world.closure_target(closure, *kind)?;
        (target.declaration, target.type_args)
    } else if method_arity == 0 {
        // Builtin recipes and Fn item providers already have exact
        // canonical dispatch services. No invented generic actual is needed.
        let call = TraitCall {
            trait_ref: application.clone(),
            item,
            method_args: Arc::from([]),
        };
        let resolved = world.resolve_trait_call(&call, &facts)?;
        let target = match resolved.callee() {
            Callee::Direct(target) => target.clone(),
            Callee::Untupled(call) => world.untupled_target(call, &facts)?.target,
            _ => {
                return Err(FactsError::unsupported(
                    budget,
                    "unsupported: opaque source provider domain remains abstract",
                ));
            }
        };
        (target.declaration, target.type_args)
    } else {
        return Err(FactsError::unsupported(
            budget,
            "unsupported: generic opaque source provider domain remains abstract",
        ));
    };
    let provider = world.callable_header(declaration)?;
    if provider.schema.env.total_arity()
        != prefix
            .len()
            .checked_add(method_arity)
            .ok_or_else(|| FactsError::resource(budget, "opaque source provider arity overflow"))?
    {
        return Err(FactsError::invalid(
            budget,
            "opaque source provider changed canonical method binder arity",
        ));
    }
    retain::<ProviderDomainSeed>(budget, 1)?;
    Ok(ProviderDomainSeed {
        facts,
        application: application.clone(),
        item,
        membership,
        declaration,
        prefix,
        method_arity,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::{
        keys,
        memory_layout::domain::compact::{closed, closed_engine_fixture},
    };
    use crate::engine::ItemKey;

    #[test]
    fn alignment_domain_opaque_capture_closure_provider_preserves_exact_membership() {
        let (engine, budget, plan, root) = closed_engine_fixture(
            r#"fn lift()->impl Fn(u16)->u32{|x:u16|x as u32}fn compose<A,B,C>(f:impl Fn(A)->B,g:impl Fn(B)->C)->impl Fn(A)->C{move|x|g(f(x))}fn main(){let h=compose(|x:u8|x as u16,lift());println!("{}",h(3u8));}"#,
            "main",
            Arc::from([]),
        );
        let main = engine
            .fact_of::<closed::Header>(
                &keys::closed_domain::ClosedDomain::header(&root, &budget)
                    .unwrap()
                    .key(),
                0,
            )
            .unwrap();
        let target = main
            .targets()
            .find(|target| {
                matches!(plan.persistent_decl_path(target.declaration).unwrap().item(),
                crate::compiler::frontend::PersistentDeclItem::Name(name) if name == "compose")
            })
            .unwrap()
            .clone();
        let facts = plan.instance_facts(&target).unwrap();
        let template = engine
            .fact_of::<crate::compiler::TemplateBodyFact>(
                &keys::Item::<{ crate::compiler::TEMPLATE_BODY }>(
                    plan.callable_index(target.declaration).unwrap(),
                )
                .key(),
                0,
            )
            .unwrap();
        let publication = template.publication.as_ref().unwrap();
        let (opaque, value) = template
            .checked
            .exprs
            .iter()
            .find_map(|expression| {
                if let ir::ExprKind::OpaqueWrap { opaque, value } = &expression.kind {
                    Some((opaque, value))
                } else {
                    None
                }
            })
            .unwrap();
        let mut context = plan.as_ref().clone();
        let authored = crate::compiler::nominal_realization::substitution::substitute_ty(
            &template.checked.exprs[value.0].ty,
            publication.facts().env(),
            &target.type_args,
            ir::FactsCtx::Validated(&facts),
            &mut context,
            &budget,
            Some(engine.authority()),
        )
        .unwrap();
        let public = crate::compiler::nominal_realization::substitution::substitute_ty(
            &Ty::Opaque(opaque.clone()),
            publication.facts().env(),
            &target.type_args,
            ir::FactsCtx::Validated(&facts),
            &mut context,
            &budget,
            Some(engine.authority()),
        )
        .unwrap();
        let Ty::Opaque(public) = public else {
            panic!("lost original opaque result")
        };
        let providers = bound_providers(facts.clone(), &public, &authored).unwrap();
        let seed = providers
            .iter()
            .find_map(|provider| {
                if let BoundProvider::Concrete { seed, .. } = provider {
                    Some(seed)
                } else {
                    None
                }
            })
            .unwrap();
        seed.validate().unwrap();
        assert!(matches!(seed.application.self_ty, Ty::Closure(_)));
        let mut foreign = seed.clone();
        foreign.application.declaration.index ^= 1;
        assert!(foreign.validate().is_err());
        let mut reordered = seed.clone();
        let prefix = Arc::make_mut(&mut reordered.prefix);
        assert_ne!(prefix[0], prefix[1]);
        prefix.swap(0, 1);
        assert!(reordered.validate().is_err());
        let universal = plan.callable_facts(target.declaration).unwrap();
        assert!(capture(universal, &seed.application, seed.item).is_err());
    }
}
