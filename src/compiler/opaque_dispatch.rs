//! Authorization-specific late dispatch for original opaque bound calls.
//! No hidden representation is read while source CHECK is being admitted.
use super::*;

const VERSION: u64 = 4;

/// Recognize only an opaque type beneath an exact ordered reference chain.
/// This never strips the chain from the dispatch goal, key or authorization.
pub(super) fn opaque_receiver<'a>(
    mut ty: &'a ir::Ty,
    budget: &ir::PlanBudget,
) -> Result<Option<&'a Arc<ir::OpaqueTy>>, Error> {
    let mut depth = 0;
    loop {
        budget.check_depth(depth).map_err(Error::new)?;
        budget.charge_work(1).map_err(Error::new)?;
        match ty {
            ir::Ty::Opaque(opaque) => return Ok(Some(opaque)),
            ir::Ty::Ref { referent, .. } => {
                ty = referent;
                depth += 1;
            }
            _ => return Ok(None),
        }
    }
}

fn physical_receiver(
    receiver: &ir::Ty,
    representations: &opaque_instances::Representations,
    budget: &ir::PlanBudget,
) -> Result<ir::Ty, Error> {
    let opaque = opaque_receiver(receiver, budget)?
        .ok_or_else(|| Error::new("opaque dispatch lost its exact receiver"))?;
    let family = representations.normalize(&ir::Ty::Opaque(opaque.clone()), budget)?;
    if !ir::carrier_locations(&family, budget)
        .map_err(Error::new)?
        .is_empty()
    {
        return Err(Error::new(
            "unsupported: opaque physical dispatch requires proven carrier-empty hidden type",
        ));
    }
    representations.normalize(receiver, budget)
}

/// A physical target is usable only together with this original occurrence.
/// This payload deliberately retains no publication, facts, world or CFG.
#[derive(Clone, Debug)]
pub(super) struct OpaqueCallAuthorization {
    publication: source_replay::OriginalBodyId,
    caller: ir::CallTarget,
    occurrence: ir::ExprId,
    public: ir::TraitCall,
    public_signature: ir::Signature,
    receiver: ir::Ty,
    hidden: ir::Ty,
    physical: ir::CallTarget,
    physical_signature: ir::Signature,
}

impl OpaqueCallAuthorization {
    pub(super) fn occurrence(&self) -> ir::ExprId {
        self.occurrence
    }
    pub(super) fn signature(&self) -> &ir::Signature {
        &self.physical_signature
    }
    pub(super) fn target(&self) -> &ir::CallTarget {
        &self.physical
    }

    /// Consumers must supply graph-read authority again. An identity token or
    /// structurally matching hidden type alone never authorizes a physical use.
    pub(super) fn replay(
        &self,
        publication: &Arc<source_replay::CanonicalSourcePublication>,
        caller: &ir::CallTarget,
        occurrence: ir::ExprId,
        representations: &opaque_instances::Representations,
        context: &frontend::PlanCtx,
        budget: &ir::PlanBudget,
    ) -> Result<(), Error> {
        if publication.identity() != &self.publication
            || caller != &self.caller
            || occurrence != self.occurrence
            || publication.facts().declaration() != caller.declaration
        {
            return Err(Error::new(
                "opaque dispatch belongs to another original publication or occurrence",
            ));
        }
        let source = publication.source();
        let (call, signature) = original_occurrence(source, occurrence)?;
        if call != &self.public || signature != &self.public_signature {
            return Err(Error::new(
                "opaque dispatch public goal or source signature was replaced",
            ));
        }
        ir::validate_callee(
            &ir::Callee::TraitItem(call.clone()),
            signature,
            ir::FactsCtx::from_validated(publication.facts()),
        )
        .map_err(Error::new)?;
        let mut specialization = opaque_specialization::Specialization::new(
            publication,
            caller,
            context,
            budget,
            representations.engine(),
        )?;
        let (ir::Callee::TraitItem(specialized), specialized_signature) =
            specialization.call(occurrence)?
        else {
            return Err(Error::new("opaque dispatch substituted another call kind"));
        };
        if !context
            .trait_world()
            .map_err(Error::new)?
            .types_equal(
                std::slice::from_ref(&specialized.trait_ref.self_ty),
                std::slice::from_ref(&self.receiver),
            )
            .map_err(|error| source_fact_error(error, budget))?
        {
            return Err(Error::new(
                "opaque dispatch changed exact specialized receiver",
            ));
        }
        let hidden = physical_receiver(&specialized.trait_ref.self_ty, representations, budget)?;
        if !context
            .trait_world()
            .map_err(Error::new)?
            .types_equal(
                std::slice::from_ref(&hidden),
                std::slice::from_ref(&self.hidden),
            )
            .map_err(|error| Error::new(error.into_legacy_diagnostic(budget)))?
        {
            return Err(Error::new(
                "opaque dispatch hidden representation was replaced",
            ));
        }
        let closed = context.instance_facts(caller).map_err(Error::new)?;
        if closed.phase() != traits::FactsPhase::Closed
            || closed.target() != Some(caller)
            || !Arc::ptr_eq(closed.world(), publication.facts().world())
            || !closed.budget().same_authority(budget)
        {
            return Err(Error::new(
                "opaque physical replay changed world, target, phase or budget authority",
            ));
        }
        let physical_call = ir::TraitCall {
            trait_ref: ir::TraitRef {
                self_ty: hidden,
                ..specialized.trait_ref.clone()
            },
            item: specialized.item,
            method_args: specialized.method_args.clone(),
        };
        let resolved = closed
            .service()
            .resolve_trait_call(&physical_call, &closed)
            .map_err(|error| Error::new(error.to_string()))?;
        if resolved.callee() != &ir::Callee::Direct(self.physical.clone())
            || resolved.signature() != &self.physical_signature
        {
            return Err(Error::new(
                "opaque physical target or signature failed canonical replay",
            ));
        }
        opaque_physical::require_preserved_boundary(
            &specialized_signature,
            &self.physical_signature,
            closed.world(),
            representations,
            budget,
        )
        .map_err(Error::new)?;
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub(super) struct PhysicalCalleeReady {
    target: ir::CallTarget,
}
impl PhysicalCalleeReady {
    pub(super) fn target(&self) -> &ir::CallTarget {
        &self.target
    }
}

/// Every public trait and method actual participates in the canonical key.
pub(super) fn key(
    caller: &ir::CallTarget,
    occurrence: ir::ExprId,
    call: &ir::TraitCall,
    budget: &ir::PlanBudget,
) -> Result<Key, Error> {
    if opaque_receiver(&call.trait_ref.self_ty, budget)?.is_none() {
        return Err(Error::new(
            "opaque dispatch requires the exact public opaque receiver",
        ));
    }
    let caller = instances::graph_key::encode(caller, budget).map_err(Error::new)?;
    budget
        .retain_bytes(2 * std::mem::size_of::<ir::Ty>() + 2 * std::mem::size_of::<usize>())
        .map_err(Error::new)?;
    let mut receiver_args = Vec::new();
    receiver_args
        .try_reserve_exact(1)
        .map_err(|_| Error::new("opaque receiver key allocation failed"))?;
    receiver_args.push(call.trait_ref.self_ty.clone());
    let receiver = instances::graph_key::encode(
        &ir::CallTarget {
            declaration: call.trait_ref.declaration,
            type_args: receiver_args.into(),
        },
        budget,
    )
    .map_err(Error::new)?;
    let trait_args = instances::graph_key::encode(
        &ir::CallTarget {
            declaration: call.trait_ref.declaration,
            type_args: call.trait_ref.args.clone(),
        },
        budget,
    )
    .map_err(Error::new)?;
    let method_args = instances::graph_key::encode(
        &ir::CallTarget {
            declaration: call.item,
            type_args: call.method_args.clone(),
        },
        budget,
    )
    .map_err(Error::new)?;
    let count = [
        caller.len(),
        receiver.len(),
        trait_args.len(),
        method_args.len(),
    ]
    .into_iter()
    .try_fold(11usize, |total, count| total.checked_add(count))
    .ok_or_else(|| Error::new("opaque dispatch key size overflow"))?;
    budget.charge_work(count).map_err(Error::new)?;
    budget
        .retain_bytes(
            count
                .checked_mul(std::mem::size_of::<u64>())
                .ok_or_else(|| Error::new("opaque dispatch key byte overflow"))?,
        )
        .map_err(Error::new)?;
    let mut words = Vec::new();
    words
        .try_reserve_exact(count)
        .map_err(|_| Error::new("opaque dispatch key allocation failed"))?;
    words.extend([VERSION, caller.len() as u64]);
    words.extend(caller);
    words.extend([
        occurrence.0 as u64,
        0, // ordinary source-call capability, not a closure invocation mode
        call.trait_ref.declaration.snapshot.graph_word(),
        call.trait_ref.declaration.index,
        call.item.snapshot.graph_word(),
        call.item.index,
        receiver.len() as u64,
    ]);
    words.extend(receiver);
    for encoded in [trait_args, method_args] {
        words.push(encoded.len() as u64);
        words.extend(encoded);
    }
    crate::compiler::keys::words(OPAQUE_CALL_DISPATCH, words)
}

/// Demand physical targets only through per-occurrence graph authorization.
/// This extends one instance frontier and never scans unrelated declarations.
pub(super) async fn extend_instance_demands(
    provider: &CompilerProvider,
    cx: &Context,
    target: &ir::CallTarget,
    template: &TemplateBodyFact,
    types: &frontend::PlanCtx,
    demands: &mut instances::DirectDemands,
) -> Result<(), Error> {
    let mut has_opaque_actual = false;
    for actual in target.type_args.iter() {
        if opaque_instances::contains(actual, &provider.0.budget)? {
            has_opaque_actual = true;
            break;
        }
    }
    if let Some(publication) = template.publication.as_ref() {
        provider
            .0
            .budget
            .charge_work(publication.source().exprs.len())
            .map_err(Error::new)?;
    }
    let mut has_source_receiver = false;
    if !has_opaque_actual && let Some(publication) = template.publication.as_ref() {
        for expression in &publication.source().exprs {
            if let ir::ExprKind::Call {
                target: ir::Callee::TraitItem(call),
                ..
            } = &expression.kind
                && opaque_receiver(&call.trait_ref.self_ty, &provider.0.budget)?.is_some()
            {
                has_source_receiver = true;
                break;
            }
        }
    }
    if !has_opaque_actual && !has_source_receiver {
        return Ok(());
    }
    let publication = template
        .publication
        .as_ref()
        .ok_or_else(|| Error::new("opaque instance dispatch lost source publication"))?;
    let mut specialization = opaque_specialization::Specialization::new(
        publication,
        target,
        types,
        &provider.0.budget,
        cx.authority(),
    )?;
    let (mut pending, _storage) = provider.scratch_vec(publication.source().exprs.len())?;
    for (index, expression) in publication.source().exprs.iter().enumerate() {
        provider.0.budget.charge_work(1).map_err(Error::new)?;
        if !matches!(
            expression.kind,
            ir::ExprKind::Call {
                target: ir::Callee::TraitItem(_),
                ..
            }
        ) {
            continue;
        }
        let site = ir::ExprId(index);
        if let (ir::Callee::TraitItem(call), _) = specialization.call(site)?
            && opaque_receiver(&call.trait_ref.self_ty, &provider.0.budget)?.is_some()
        {
            pending.push((
                site,
                cx.request(key(target, site, &call, &provider.0.budget)?)
                    .get(0),
            ));
        }
    }
    let (mut proofs, _proof_storage) = provider.scratch_vec(pending.len())?;
    let (mut roots, _root_storage) = provider.scratch_vec(pending.len())?;
    for (site, handle) in pending {
        let authorization: Arc<OpaqueCallAuthorization> = cx.read_fact(handle).await?;
        roots.push(authorization.receiver.clone());
        proofs.push((site, authorization));
    }
    let representations = opaque_instances::read(&Facts { cx }, &roots, &provider.0.budget).await?;
    for (site, authorization) in proofs {
        authorization.replay(
            publication,
            target,
            site,
            &representations,
            types,
            &provider.0.budget,
        )?;
        demands
            .call(authorization.target().clone(), &provider.0.budget)
            .map_err(Error::new)?;
    }
    Ok(())
}

fn original_occurrence(
    source: &ir::CheckedFunction,
    occurrence: ir::ExprId,
) -> Result<(&ir::TraitCall, &ir::Signature), Error> {
    let expression = source
        .exprs
        .get(occurrence.0)
        .ok_or_else(|| Error::new("opaque dispatch occurrence is outside its original body"))?;
    match &expression.kind {
        ir::ExprKind::Call {
            target: ir::Callee::TraitItem(call),
            signature,
            ..
        } => Ok((call, signature)),
        _ => Err(Error::new(
            "opaque dispatch occurrence is not an original trait call",
        )),
    }
}

/// The source publication and admission prerequisite are independently demanded.
/// The physical body is not demanded until output zero seals the complete proof.
pub(super) async fn produce(
    provider: &CompilerProvider,
    cx: &Context,
    requested: &Key,
) -> Result<(), Error> {
    let facts = Facts { cx };
    let types = facts.types().await?;
    let words = &requested.params;
    if words.first() != Some(&VERSION) {
        return Err(Error::new("unsupported opaque dispatch key version"));
    }
    let len = usize::try_from(
        *words
            .get(1)
            .ok_or_else(|| Error::new("truncated opaque dispatch key"))?,
    )
    .map_err(|_| Error::new("opaque caller frame exceeds address space"))?;
    let end = 2usize
        .checked_add(len)
        .ok_or_else(|| Error::new("opaque caller frame overflow"))?;
    let caller_words = words
        .get(2..end)
        .ok_or_else(|| Error::new("truncated opaque caller frame"))?;
    let caller = facts
        .decode_instance(caller_words, &types, &provider.0.budget)
        .await?;
    let occurrence = ir::ExprId(
        usize::try_from(
            *words
                .get(end)
                .ok_or_else(|| Error::new("missing opaque call occurrence"))?,
        )
        .map_err(|_| Error::new("opaque occurrence exceeds address space"))?,
    );
    let index = types
        .callable_index(caller.declaration)
        .map_err(Error::new)?;
    let template_handle = cx
        .request(crate::compiler::keys::item(TEMPLATE_BODY, index))
        .get(0);
    let admission_handle = cx
        .request(crate::compiler::keys::item(SOURCE_ADMISSION, index))
        .get(0);
    let template: Arc<TemplateBodyFact> = cx.read_fact(template_handle).await?;
    cx.read(admission_handle).await?;
    let source = provider.validated_body(&types, index, &template)?;
    let publication = template
        .publication
        .as_ref()
        .ok_or_else(|| Error::new("opaque dispatch has no canonical original publication"))?;
    let (public, signature) = original_occurrence(&source, occurrence)?;
    let mut specialization = opaque_specialization::Specialization::new(
        publication,
        &caller,
        &types,
        &provider.0.budget,
        cx.authority(),
    )?;
    let (ir::Callee::TraitItem(specialized), specialized_signature) =
        specialization.call(occurrence)?
    else {
        return Err(Error::new(
            "opaque dispatch original call changed kind after substitution",
        ));
    };
    if key(&caller, occurrence, &specialized, &provider.0.budget)? != *requested {
        return Err(Error::new(
            "opaque dispatch authorization does not match original call metadata",
        ));
    }
    ir::validate_callee(
        &ir::Callee::TraitItem(public.clone()),
        signature,
        ir::FactsCtx::from_validated(publication.facts()),
    )
    .map_err(|error| Error::new(error.to_string()))?;
    let representations = opaque_instances::read(
        &facts,
        std::slice::from_ref(&specialized.trait_ref.self_ty),
        &provider.0.budget,
    )
    .await?;
    let hidden = physical_receiver(
        &specialized.trait_ref.self_ty,
        &representations,
        &provider.0.budget,
    )?;
    let physical_call = ir::TraitCall {
        trait_ref: ir::TraitRef {
            declaration: specialized.trait_ref.declaration,
            self_ty: hidden.clone(),
            args: specialized.trait_ref.args.clone(),
        },
        item: specialized.item,
        method_args: specialized.method_args.clone(),
    };
    let closed = types.instance_facts(&caller).map_err(Error::new)?;
    let resolved = closed
        .service()
        .resolve_trait_call(&physical_call, &closed)
        .map_err(|error| Error::new(error.to_string()))?;
    let ir::Callee::Direct(physical) = resolved.callee() else {
        return Err(Error::new(
            "unsupported: opaque bound did not resolve to a closed physical target",
        ));
    };
    opaque_physical::require_preserved_boundary(
        &specialized_signature,
        resolved.signature(),
        closed.world(),
        &representations,
        &provider.0.budget,
    )
    .map_err(Error::new)?;
    if physical == &caller {
        return Err(Error::new(
            "unsupported: recursive opaque physical dispatch",
        ));
    }
    let world = types.trait_world().map_err(Error::new)?;
    let physical = physical.clone();
    let proof = OpaqueCallAuthorization {
        publication: publication.identity().clone(),
        caller,
        occurrence,
        public: public.clone(),
        public_signature: signature.clone(),
        receiver: specialized.trait_ref.self_ty.clone(),
        hidden,
        physical: physical.clone(),
        physical_signature: resolved.signature().clone(),
    };
    let physical_signature = proof.physical_signature.clone();
    proof.replay(
        publication,
        &proof.caller,
        occurrence,
        &representations,
        &types,
        &provider.0.budget,
    )?;
    cx.publish_fact(0, proof)?;
    let body_key = crate::compiler::keys::words(
        CANONICAL_BODY,
        instances::graph_key::encode(&physical, &provider.0.budget).map_err(Error::new)?,
    )?;
    let body: Arc<InstanceBodyFact> = facts.get(body_key).await?;
    if body.facts.phase() != traits::FactsPhase::Closed || body.facts.target() != Some(&physical) {
        return Err(Error::new(
            "opaque physical body does not carry the selected closed target",
        ));
    }
    let body_params = body
        .checked
        .params
        .iter()
        .map(|id| {
            body.checked
                .locals
                .get(id.0)
                .map(|local| local.ty.clone())
                .ok_or_else(|| Error::new("physical body parameter is outside its locals"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if !world
        .types_equal(&body_params, &physical_signature.params)
        .map_err(|e| Error::new(e.to_string()))?
        || !world
            .types_equal(
                std::slice::from_ref(&body.checked.result),
                &[physical_signature.result],
            )
            .map_err(|e| Error::new(e.to_string()))?
    {
        return Err(Error::new(
            "selected physical body signature differs from dispatch proof",
        ));
    }
    cx.publish_fact(1, PhysicalCalleeReady { target: physical })
}

impl crate::engine::Fact for OpaqueCallAuthorization {}

impl crate::engine::Fact for PhysicalCalleeReady {}
