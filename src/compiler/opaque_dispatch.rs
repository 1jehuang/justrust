//! Authorization-specific late dispatch for original opaque bound calls.
//! No hidden representation is read while source CHECK is being admitted.
use super::*;

const VERSION: u64 = 1;

/// A physical target is usable only together with this original occurrence.
/// This payload deliberately retains no publication, facts, world or CFG.
#[derive(Clone, Debug)]
pub(super) struct OpaqueCallAuthorization {
    publication: source_replay::OriginalBodyId,
    caller: ir::CallTarget,
    occurrence: ir::ExprId,
    public: ir::TraitCall,
    public_signature: ir::Signature,
    opaque: Arc<ir::OpaqueTy>,
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
        if call != &self.public
            || signature != &self.public_signature
            || call.trait_ref.self_ty != ir::Ty::Opaque(self.opaque.clone())
        {
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
        let hidden = representations.normalize(&call.trait_ref.self_ty, budget)?;
        if !ir::identical_type(&hidden, &self.hidden, budget).map_err(Error::new)? {
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
                ..call.trait_ref.clone()
            },
            item: call.item,
            method_args: call.method_args.clone(),
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

/// The first slice admits scalar Read-like methods only. Reject metadata we
/// cannot encode rather than allowing two different authorizations to merge.
pub(super) fn key(
    caller: &ir::CallTarget,
    occurrence: ir::ExprId,
    call: &ir::TraitCall,
    budget: &ir::PlanBudget,
) -> Result<Key, Error> {
    if !call.trait_ref.args.is_empty() || !call.method_args.is_empty() {
        return Err(Error::new(
            "unsupported: opaque dispatch with trait or method actuals",
        ));
    }
    let ir::Ty::Opaque(opaque) = &call.trait_ref.self_ty else {
        return Err(Error::new(
            "opaque dispatch requires the exact public opaque receiver",
        ));
    };
    let caller = instances::graph_key::encode(caller, budget).map_err(Error::new)?;
    let opaque = instances::graph_key::encode_opaque(opaque, budget).map_err(Error::new)?;
    let mut words = vec![VERSION, caller.len() as u64];
    words.extend(caller);
    words.extend([
        occurrence.0 as u64,
        0, // ordinary source-call capability, not a closure invocation mode
        call.trait_ref.declaration.snapshot.graph_word(),
        u64::from(call.trait_ref.declaration.index),
        call.item.snapshot.graph_word(),
        u64::from(call.item.index),
        opaque.len() as u64,
    ]);
    words.extend(opaque);
    Ok(Key::new(OPAQUE_CALL_DISPATCH, words))
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
    let caller = instances::graph_key::decode(caller_words, &types, &provider.0.budget)
        .map_err(Error::new)?;
    let occurrence = ir::ExprId(
        usize::try_from(
            *words
                .get(end)
                .ok_or_else(|| Error::new("missing opaque call occurrence"))?,
        )
        .map_err(|_| Error::new("opaque occurrence exceeds address space"))?,
    );
    if !caller.type_args.is_empty() {
        return Err(Error::new(
            "unsupported: generic caller opaque dispatch requires source substitution replay",
        ));
    }
    let index = types
        .callable_index(caller.declaration)
        .map_err(Error::new)?;
    let template_handle = cx.request(Facts::item(TEMPLATE_BODY, index)).get(0);
    let admission_handle = cx.request(Facts::item(SOURCE_ADMISSION, index)).get(0);
    let template: Arc<TemplateBodyFact> = cx.read_fact(template_handle).await?;
    cx.read(admission_handle).await?;
    let source = provider.validated_body(&types, index, &template)?;
    let publication = template
        .publication
        .as_ref()
        .ok_or_else(|| Error::new("opaque dispatch has no canonical original publication"))?;
    let (public, signature) = original_occurrence(&source, occurrence)?;
    if key(&caller, occurrence, public, &provider.0.budget)? != *requested {
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
    let ir::Ty::Opaque(opaque) = &public.trait_ref.self_ty else {
        unreachable!()
    };
    let representations = opaque_instances::read(
        &facts,
        &[public.trait_ref.self_ty.clone()],
        &provider.0.budget,
    )
    .await?;
    let hidden = representations.normalize(&public.trait_ref.self_ty, &provider.0.budget)?;
    if !ir::carrier_locations(&hidden, &provider.0.budget)
        .map_err(Error::new)?
        .is_empty()
    {
        return Err(Error::new(
            "unsupported: opaque physical dispatch requires proven carrier-empty hidden type",
        ));
    }
    let physical_call = ir::TraitCall {
        trait_ref: ir::TraitRef {
            declaration: public.trait_ref.declaration,
            self_ty: hidden.clone(),
            args: public.trait_ref.args.clone(),
        },
        item: public.item,
        method_args: public.method_args.clone(),
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
    if physical == &caller {
        return Err(Error::new(
            "unsupported: recursive opaque physical dispatch",
        ));
    }
    let params = signature
        .params
        .iter()
        .map(|ty| representations.normalize(ty, &provider.0.budget))
        .collect::<Result<Vec<_>, _>>()?;
    let result = representations.normalize(&signature.result, &provider.0.budget)?;
    let world = types.trait_world().map_err(Error::new)?;
    if !world
        .types_equal(&params, &resolved.signature().params)
        .map_err(|e| Error::new(e.to_string()))?
        || !world
            .types_equal(&[result], &[resolved.signature().result.clone()])
            .map_err(|e| Error::new(e.to_string()))?
    {
        return Err(Error::new(
            "opaque physical signature does not realize the original public call",
        ));
    }
    let physical = physical.clone();
    let proof = OpaqueCallAuthorization {
        publication: publication.identity().clone(),
        caller,
        occurrence,
        public: public.clone(),
        public_signature: signature.clone(),
        opaque: opaque.clone(),
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
    let body_key = Key::new(
        CANONICAL_BODY,
        instances::graph_key::encode(&physical, &provider.0.budget).map_err(Error::new)?,
    );
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
            .types_equal(&[body.checked.result.clone()], &[physical_signature.result])
            .map_err(|e| Error::new(e.to_string()))?
    {
        return Err(Error::new(
            "selected physical body signature differs from dispatch proof",
        ));
    }
    cx.publish_fact(1, PhysicalCalleeReady { target: physical })
}
