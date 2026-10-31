//! Per-body physical normalization. Original source and universal CFG are never
//! rewritten. Every changed call and defining pack is replayed before cloning.
use super::*;

#[path = "opaque_physical/carriers.rs"]
mod carriers;
pub(super) use carriers::require_preserved_boundary;
pub(super) use carriers::{PhysicalCarrierFlow, PhysicalCarrierIdentity};

#[derive(Clone, Debug)]
struct DirectPhysicalCall {
    publication: source_replay::OriginalBodyId,
    occurrence: ir::ExprId,
    source_target: ir::CallTarget,
    target: ir::CallTarget,
    public_signature: ir::Signature,
    physical_signature: ir::Signature,
}
impl DirectPhysicalCall {
    fn seal(
        publication: &Arc<source_replay::CanonicalSourcePublication>,
        occurrence: ir::ExprId,
        representations: &opaque_instances::Representations,
        context: &frontend::PlanCtx,
        budget: &ir::PlanBudget,
        specialization: &mut opaque_specialization::Specialization<'_>,
    ) -> Result<Self, Error> {
        specialization.authenticate_context(publication, context, budget)?;
        let expression = publication
            .source()
            .exprs
            .get(occurrence.0)
            .ok_or_else(|| Error::new("direct physical ABI occurrence is outside original body"))?;
        let ir::ExprKind::Call {
            target: ir::Callee::Direct(target),
            signature,
            ..
        } = &expression.kind
        else {
            return Err(Error::new(
                "physical direct ABI requires an original direct-call occurrence",
            ));
        };
        let source_target = target.clone();
        // The shared specialization authenticates this exact original call
        // before mapping its ABI. It retains separate original/closed scratch
        // for this body, not a fresh proof arena for each direct occurrence.
        let (ir::Callee::Direct(target), specialized_signature) =
            specialization.call(occurrence)?
        else {
            return Err(Error::new(
                "physical direct call changed its original callee kind",
            ));
        };
        // Keep the public instance identity. Its object producer performs the
        // same authenticated physical normalization, so callers and callees
        // agree on the ABI without creating a hidden-type instance alias.
        for actual in target.type_args.iter() {
            representations.normalize(actual, budget)?;
        }
        let params = specialized_signature
            .params
            .iter()
            .map(|ty| representations.normalize(ty, budget))
            .collect::<Result<Vec<_>, _>>()?;
        let result = representations.normalize(&specialized_signature.result, budget)?;
        let physical_signature = ir::Signature {
            params,
            result,
            ref_contract: specialized_signature.ref_contract.clone(),
        };
        carriers::require_preserved_boundary(
            &specialized_signature,
            &physical_signature,
            context.trait_world().map_err(Error::new)?.as_ref(),
            representations,
            budget,
        )
        .map_err(Error::new)?;
        Ok(Self {
            publication: publication.identity().clone(),
            occurrence,
            source_target,
            target,
            public_signature: signature.clone(),
            physical_signature,
        })
    }
}

/// Immutable description of the exact public-to-physical type correspondence.
/// No original publication, CFG, facts, engine handle, or world is retained.
#[derive(Clone, Debug)]
pub(super) struct PhysicalNormalizationProof {
    publication: source_replay::OriginalBodyId,
    target: ir::CallTarget,
    local_types: Arc<[ir::Ty]>,
    result: ir::Ty,
    calls: Arc<[Arc<opaque_dispatch::OpaqueCallAuthorization>]>,
    direct_calls: Arc<[DirectPhysicalCall]>,
    packs: Arc<[(ir::ExprId, ir::OpaquePackProof)]>,
}

/// A draft cannot enter ordinary close/emission APIs. The explicit physical
/// shape validator must seal it after checking its complete correspondence.
pub(super) struct PhysicalCfgDraft {
    function: cfg::Function,
    proof: Arc<PhysicalNormalizationProof>,
}

impl PhysicalCfgDraft {
    pub(super) fn function(&self) -> &cfg::Function {
        &self.function
    }
    pub(super) fn seal<'a>(
        &mut self,
        publication: &'a Arc<source_replay::CanonicalSourcePublication>,
        original: &'a cfg::Function,
        representations: &'a opaque_instances::Representations,
        context: &'a frontend::PlanCtx,
        closed: &traits::ValidatedFacts,
        budget: &'a ir::PlanBudget,
    ) -> Result<PhysicalValidation<'a>, String> {
        let mut validation = PhysicalValidation::new(
            self.proof.clone(),
            publication,
            original,
            representations,
            context,
            budget,
        )?;
        validation.validate_mapping(&self.function, closed)?;
        self.function
            .seal_physical_shape(closed, self.proof.clone(), budget)?;
        validation.validate_cfg(&self.function, closed)?;
        validation.sealed_shape = self.function.lowering_proof.clone();
        Ok(validation)
    }
    #[cfg(test)]
    pub(super) fn validation<'a>(
        &'a self,
        publication: &'a Arc<source_replay::CanonicalSourcePublication>,
        original: &'a cfg::Function,
        representations: &'a opaque_instances::Representations,
        context: &'a frontend::PlanCtx,
        budget: &'a ir::PlanBudget,
    ) -> Result<PhysicalValidation<'a>, String> {
        let mut validation = PhysicalValidation::new(
            self.proof.clone(),
            publication,
            original,
            representations,
            context,
            budget,
        )?;
        let closed = context.instance_facts(&self.proof.target)?;
        validation.validate_cfg(&self.function, &closed)?;
        validation.sealed_shape = self.function.lowering_proof.clone();
        Ok(validation)
    }
}

/// Ephemeral borrowed authority. It is never published or retained in a proof.
/// Ordinary CFG validation cannot manufacture this context from a type map.
pub(super) struct PhysicalValidation<'a> {
    proof: Arc<PhysicalNormalizationProof>,
    publication: &'a Arc<source_replay::CanonicalSourcePublication>,
    expected: PhysicalCfgDraft,
    sealed_shape: Option<Arc<cfg::GeneratedCfgShape>>,
    original: &'a cfg::Function,
    representations: &'a opaque_instances::Representations,
    budget: &'a ir::PlanBudget,
}
impl<'a> PhysicalValidation<'a> {
    fn new(
        proof: Arc<PhysicalNormalizationProof>,
        publication: &'a Arc<source_replay::CanonicalSourcePublication>,
        original: &'a cfg::Function,
        representations: &'a opaque_instances::Representations,
        context: &'a frontend::PlanCtx,
        budget: &'a ir::PlanBudget,
    ) -> Result<Self, String> {
        // Replay once from the immutable original inputs. Every later check
        // compares its candidate with this independent, immutable result, not
        // with a previously accepted candidate. This capability stays local to
        // the consumer and is never stored in a graph fact.
        let expected = normalize(
            publication,
            &proof.target,
            original,
            representations,
            &proof.calls,
            context,
            budget,
        )
        .map_err(|error| error.to_string())?;
        Ok(Self {
            proof,
            publication,
            expected,
            sealed_shape: None,
            original,
            representations,
            budget,
        })
    }

    pub(super) fn validate_cfg(
        &self,
        candidate: &cfg::Function,
        facts: &traits::ValidatedFacts,
    ) -> Result<(), String> {
        self.validate_mapping(candidate, facts)?;
        if let Some(shape) = &self.sealed_shape {
            // The private snapshot was checked against the independently
            // normalized body before this borrowed context was returned.
            // Mapping still checks every candidate field on every use.
            if candidate
                .lowering_proof
                .as_ref()
                .is_some_and(|candidate| Arc::ptr_eq(candidate, shape))
            {
                return Ok(());
            }
            return Err("physical CFG changed its authenticated shape snapshot".into());
        }
        candidate.validate_physical_shape(&self.proof, facts, self.budget)
    }
    fn validate_mapping(
        &self,
        candidate: &cfg::Function,
        facts: &traits::ValidatedFacts,
    ) -> Result<(), String> {
        if facts.phase() != traits::FactsPhase::Closed
            || facts.target() != Some(&self.proof.target)
            || self.publication.identity() != &self.proof.publication
            || !Arc::ptr_eq(facts.world(), self.publication.facts().world())
            || !facts.budget().same_authority(self.budget)
        {
            return Err(
                "physical CFG changed original publication, target or world authority".into(),
            );
        }
        let expected = &self.expected;
        if self.proof.local_types != expected.proof.local_types
            || self.proof.result != expected.proof.result
            || self.proof.packs.len() != expected.proof.packs.len()
            || self.proof.direct_calls.len() != expected.proof.direct_calls.len()
        {
            return Err("physical CFG type, pack or direct-call mapping changed".into());
        }
        for ((site, pack), (expected_site, expected_pack)) in
            self.proof.packs.iter().zip(expected.proof.packs.iter())
        {
            // normalize authenticated the expected pack against the original
            // publication. Require exact immutable identities rather than
            // replaying that source for every carrier use of this CFG.
            if site != expected_site || !pack.matches_replayed(expected_pack, self.budget)? {
                return Err("physical pack occurrence mapping changed".into());
            }
        }
        for (call, other) in self
            .proof
            .direct_calls
            .iter()
            .zip(expected.proof.direct_calls.iter())
        {
            if call.publication != other.publication
                || call.occurrence != other.occurrence
                || call.source_target != other.source_target
                || call.target != other.target
                || call.public_signature != other.public_signature
                || call.physical_signature != other.physical_signature
            {
                return Err("physical direct-call ABI mapping changed".into());
            }
        }
        let expected = &expected.function;
        if candidate.runtime_sites != expected.runtime_sites
            || candidate.name != expected.name
            || candidate.params != expected.params
            || candidate.locals != expected.locals
            || candidate.blocks != expected.blocks
            || candidate.loan_sites != expected.loan_sites
            || candidate.result != expected.result
            || candidate.ref_contract != expected.ref_contract
            || candidate.entry != expected.entry
            || candidate.return_local != expected.return_local
            || !candidate.call_evidence.is_empty()
            || candidate.body_substitution.is_some()
        {
            return Err("physical CFG no longer matches exact original control, operands, loans or type mapping".into());
        }
        Ok(())
    }
    pub(super) fn validate_call(
        &self,
        occurrence: Option<ir::ExprId>,
        callee: &ir::Callee,
        signature: &ir::Signature,
    ) -> Result<(), String> {
        let site = occurrence.ok_or("physical call has no original occurrence")?;
        // `new` independently replays every call from the immutable original
        // publication into `expected`. Reuse only that private replay result,
        // never the candidate's proof or a graph-global cache. Re-deriving it
        // for each ownership/borrow pass repeats full source/module walks.
        self.budget.charge_work(self.expected.proof.calls.len())?;
        if let Some(call) = self
            .expected
            .proof
            .calls
            .iter()
            .find(|call| call.occurrence() == site)
        {
            if callee == &ir::Callee::Direct(call.target().clone()) && signature == call.signature()
            {
                return Ok(());
            }
        } else {
            self.budget
                .charge_work(self.expected.proof.direct_calls.len())?;
            if let Some(expected) = self
                .expected
                .proof
                .direct_calls
                .iter()
                .find(|call| call.occurrence == site)
                && callee == &ir::Callee::Direct(expected.target.clone())
                && signature == &expected.physical_signature
            {
                return Ok(());
            }
        }
        Err("physical call lacks exact original-occurrence ABI authorization".into())
    }
}

// Shape preflight only. The caller must first replay the original pack and
// obtain this actual family through authenticated instance substitution.
fn require_empty_physical_capture(actual: &ir::Ty, budget: &ir::PlanBudget) -> Result<(), Error> {
    if !ir::carrier_locations(actual, budget)
        .map_err(Error::new)?
        .is_empty()
    {
        return Err(Error::new(
            "unsupported: physical captured family requires an explicit reference contract",
        ));
    }
    Ok(())
}

fn specialize_marker_constants(
    function: &mut cfg::Function,
    specialization: &mut opaque_specialization::Specialization<'_>,
    budget: &ir::PlanBudget,
) -> Result<(), Error> {
    fn operand(
        operand: &mut cfg::Operand,
        specialization: &mut opaque_specialization::Specialization<'_>,
        budget: &ir::PlanBudget,
    ) -> Result<(), Error> {
        budget.charge_work(1).map_err(Error::new)?;
        if let cfg::Operand::Constant(cfg::Constant::PhantomData(marker)) = operand {
            let ir::Ty::PhantomData(mapped) =
                specialization.ty(&ir::Ty::PhantomData(marker.clone()))?
            else {
                return Err(Error::new(
                    "physical marker substitution changed its type kind",
                ));
            };
            if mapped.declaration() != marker.declaration() {
                return Err(Error::new(
                    "physical marker substitution changed canonical owner",
                ));
            }
            *marker = mapped;
        }
        Ok(())
    }
    fn rvalue(
        value: &mut cfg::Rvalue,
        specialization: &mut opaque_specialization::Specialization<'_>,
        budget: &ir::PlanBudget,
    ) -> Result<(), Error> {
        use cfg::Rvalue::*;
        match value {
            ManuallyDropTake { .. } | ManuallyDropIntoInner { .. } => {
                return Err(Error::new(
                    "unsupported: physical ManuallyDrop payload effect requires authenticated closed replay",
                ));
            }
            OpaquePack { value, .. }
            | Use(value)
            | CharToInt { value, .. }
            | U8ToChar { value }
            | PtrCast { value, .. }
            | LeadingZeros { value }
            | IntCast { value, .. }
            | FloatCast { value, .. }
            | Unary { value, .. }
            | Repeat { value, .. }
            | ArrayToSlice { reference: value }
            | RefToDyn {
                reference: value, ..
            }
            | SliceDataPointer { slice: value }
            | SliceLen { slice: value }
            | IndexCopy { index: value, .. } => operand(value, specialization, budget)?,
            SliceIndex { slice, index } => {
                operand(slice, specialization, budget)?;
                operand(index, specialization, budget)?;
            }
            Subslice {
                slice, start, end, ..
            } => {
                operand(slice, specialization, budget)?;
                operand(start, specialization, budget)?;
                if let Some(end) = end {
                    operand(end, specialization, budget)?;
                }
            }
            Binary { left, right, .. } => {
                operand(left, specialization, budget)?;
                operand(right, specialization, budget)?;
            }
            NativeSyscall(fields)
            | RawSlice { args: fields, .. }
            | Aggregate { fields, .. }
            | EnumVariant { fields, .. }
            | Intrinsic { args: fields, .. }
            | Format {
                arguments: fields, ..
            } => {
                for field in fields {
                    operand(field, specialization, budget)?;
                }
            }
            TypeAlignment(_)
            | Borrow(_)
            | Discriminant(_)
            | Println(_)
            | Output(_)
            | Str(_)
            | ByteStr(_)
            | PromotedScalar(_)
            | ByteRepeat { .. } => {}
        }
        Ok(())
    }
    for block in &mut function.blocks {
        budget.charge_work(1).map_err(Error::new)?;
        for statement in &mut block.statements {
            budget.charge_work(1).map_err(Error::new)?;
            match &mut statement.value {
                cfg::Statement::Assign { value, .. } => rvalue(value, specialization, budget)?,
                cfg::Statement::IndexAssignOp { index, value, .. }
                | cfg::Statement::IndexAssign { index, value, .. } => {
                    operand(index, specialization, budget)?;
                    operand(value, specialization, budget)?;
                }
                cfg::Statement::IndexInspect { index, .. } => {
                    operand(index, specialization, budget)?
                }
                cfg::Statement::Inspect(_)
                | cfg::Statement::StorageLive(_)
                | cfg::Statement::StorageDead(_) => {}
            }
        }
        match &mut block.terminator.value {
            cfg::Terminator::Call { args, .. } => {
                for argument in args {
                    operand(argument, specialization, budget)?;
                }
            }
            cfg::Terminator::Switch { condition, .. } => {
                operand(condition, specialization, budget)?
            }
            cfg::Terminator::Drop { .. }
            | cfg::Terminator::SwitchVariant { .. }
            | cfg::Terminator::Goto(_)
            | cfg::Terminator::Return
            | cfg::Terminator::Panic(_)
            | cfg::Terminator::Unreachable => {}
        }
    }
    Ok(())
}

/// Materialize an exactly carrier-preserving boundary from canonical
/// lowering. CFG blocks, argument snapshots, loans and evaluation order are
/// copied intact. Only types and independently authorized call/pack operations
/// can change. Unsupported operations fail without producing a physical body.
pub(super) fn normalize(
    publication: &Arc<source_replay::CanonicalSourcePublication>,
    target: &ir::CallTarget,
    original: &cfg::Function,
    representations: &opaque_instances::Representations,
    calls: &[Arc<opaque_dispatch::OpaqueCallAuthorization>],
    context: &frontend::PlanCtx,
    budget: &ir::PlanBudget,
) -> Result<PhysicalCfgDraft, Error> {
    if publication.facts().declaration() != target.declaration {
        return Err(Error::new(
            "physical normalization changed exact original source owner",
        ));
    }
    let mut specialization = opaque_specialization::Specialization::new(
        publication,
        target,
        context,
        budget,
        representations.engine(),
    )?;
    original
        .validate_source_publication(publication, budget)
        .map_err(|e| Error::new(e.to_string()))?;
    for call in calls {
        call.replay(
            publication,
            target,
            call.occurrence(),
            representations,
            context,
            budget,
        )?;
    }
    let mut output = original.clone();
    let mut operand_scratch = super::traits::TraitScratch::default();
    let mut local_types = Vec::with_capacity(original.locals.len());
    for local in &mut output.locals {
        local.ty = representations.normalize(&specialization.ty(&local.ty)?, budget)?;
        local_types.push(local.ty.clone());
    }
    output.result = representations.normalize(&specialization.ty(&original.result)?, budget)?;
    let parameters = output
        .params
        .iter()
        .map(|id| output.locals[id.0].ty.clone())
        .collect::<Vec<_>>();
    let closed = context.instance_facts(target).map_err(Error::new)?;
    let original_boundary = closed
        .carrier_contract(ir::SignatureSubject::Body)
        .map_err(|error| source_fact_error(error, budget))?;
    original_boundary
        .validate_for(&closed)
        .map_err(|error| source_fact_error(error, budget))?;
    let signature = ir::Signature {
        params: parameters,
        result: output.result.clone(),
        ref_contract: original_boundary.ref_contract().clone(),
    };
    carriers::require_preserved_boundary(
        original_boundary.physical_source_signature(),
        &signature,
        closed.world(),
        representations,
        budget,
    )
    .map_err(Error::new)?;
    output.ref_contract = signature.ref_contract;
    let mut packs = Vec::new();
    let mut direct_calls = Vec::new();
    for block in &mut output.blocks {
        for statement in &mut block.statements {
            let cfg::Statement::Assign { value, .. } = &mut statement.value else {
                continue;
            };
            match value {
                cfg::Rvalue::OpaquePack {
                    value: operand,
                    proof,
                } => {
                    let occurrence = statement
                        .origin
                        .ok_or_else(|| Error::new("physical pack lost original occurrence"))?;
                    proof
                        .validate_occurrence(publication, Some(occurrence), budget)
                        .map_err(Error::new)?;
                    let ir::Ty::Opaque(opaque) =
                        specialization.ty(&ir::Ty::Opaque(proof.opaque().clone()))?
                    else {
                        return Err(Error::new(
                            "opaque owner substitution changed its type kind",
                        ));
                    };
                    let record = representations.get(&opaque, budget)?;
                    for source in proof.captured_sources() {
                        let source_family = match &source.channel {
                            ir::CarrierChannel::ParamTail {
                                parameter,
                                prefix: 0,
                            } => ir::Ty::Param(*parameter),
                            ir::CarrierChannel::OpaqueTail { opaque, prefix: 0 }
                                if source.path == ir::CarrierPath::default()
                                    && proof.hidden() == &ir::Ty::Opaque(opaque.clone()) =>
                            {
                                ir::Ty::Opaque(opaque.clone())
                            }
                            _ => {
                                return Err(Error::new(
                                    "physical pack has an unsupported original capture channel",
                                ));
                            }
                        };
                        let actual_family = specialization.ty(&source_family)?;
                        let physical_family = representations.normalize(&actual_family, budget)?;
                        require_empty_physical_capture(&physical_family, budget)?;
                    }
                    let physical_hidden = representations.normalize(record.hidden(), budget)?;
                    if !ir::carrier_locations(&physical_hidden, budget)
                        .map_err(Error::new)?
                        .is_empty()
                    {
                        return Err(Error::new(
                            "unsupported: physical pack has hidden reference carriers",
                        ));
                    }
                    let actual = original
                        .operand_ty_with_facts(
                            operand,
                            ir::FactsCtx::from_validated(publication.facts()),
                            &mut operand_scratch,
                        )
                        .map_err(Error::new)?;
                    let actual = specialization.ty(&actual)?;
                    if !context
                        .trait_world()
                        .map_err(Error::new)?
                        .types_equal(
                            std::slice::from_ref(&actual),
                            std::slice::from_ref(record.hidden()),
                        )
                        .map_err(|error| Error::new(error.into_legacy_diagnostic(budget)))?
                    {
                        return Err(Error::new(
                            "physical pack operand differs from exact defining representation",
                        ));
                    }
                    packs.push((occurrence, proof.as_ref().clone()));
                    // This conversion is enclosed in the normalization proof.
                    // It is not admitted by ordinary CFG close or codegen.
                    *value = cfg::Rvalue::Use(operand.clone());
                }
                cfg::Rvalue::Aggregate { ty, .. }
                | cfg::Rvalue::Repeat { ty, .. }
                | cfg::Rvalue::PtrCast { to: ty, .. } => {
                    *ty = representations.normalize(&specialization.ty(ty)?, budget)?;
                }
                _ => {}
            }
        }
        if let cfg::Terminator::Call {
            target: callee,
            signature,
            ..
        } = &mut block.terminator.value
        {
            let occurrence = block
                .terminator
                .origin
                .ok_or_else(|| Error::new("physical call lost original occurrence"))?;
            match callee {
                ir::Callee::TraitItem(_)
                    if calls.iter().any(|proof| proof.occurrence() == occurrence) =>
                {
                    let authorization = calls
                        .iter()
                        .find(|proof| proof.occurrence() == occurrence)
                        .ok_or_else(|| {
                            Error::new(
                                "physical opaque call has no occurrence-specific authorization",
                            )
                        })?;
                    authorization.replay(
                        publication,
                        target,
                        occurrence,
                        representations,
                        context,
                        budget,
                    )?;
                    *callee = ir::Callee::Direct(authorization.target().clone());
                    *signature = authorization.signature().clone();
                }
                ir::Callee::Direct(_) => {
                    let direct = DirectPhysicalCall::seal(
                        publication,
                        occurrence,
                        representations,
                        context,
                        budget,
                        &mut specialization,
                    )?;
                    if callee != &ir::Callee::Direct(direct.source_target.clone())
                        || signature != &direct.public_signature
                    {
                        return Err(Error::new(
                            "physical direct call differs from original occurrence",
                        ));
                    }
                    *callee = ir::Callee::Direct(direct.target.clone());
                    *signature = direct.physical_signature.clone();
                    direct_calls.push(direct);
                }
                ir::Callee::TraitItem(_) | ir::Callee::Virtual(_) | ir::Callee::Untupled(_) => {
                    return Err(Error::new("unsupported: residual abstract physical callee"));
                }
            }
        }
    }
    // Original pack operands and call occurrences above must remain universal
    // until their source proofs have been authenticated. Only now close marker
    // constants; they retain full canonical type metadata despite zero storage.
    specialize_marker_constants(&mut output, &mut specialization, budget)?;
    // Original source evidence is retained in `original`, not relabeled as
    // closed evidence. The physical shape protocol must replay it separately.
    output.lowering_proof = None;
    output.call_evidence = Arc::from([]);
    output.body_substitution = None;
    let proof = PhysicalNormalizationProof {
        publication: publication.identity().clone(),
        target: target.clone(),
        local_types: local_types.into(),
        result: output.result.clone(),
        calls: calls.to_vec().into(),
        direct_calls: direct_calls.into(),
        packs: packs.into(),
    };
    Ok(PhysicalCfgDraft {
        function: output,
        proof: Arc::new(proof),
    })
}

#[cfg(test)]
mod captured_family_tests {
    use super::*;

    #[test]
    fn marker_constant_specialization_preserves_original_source_and_closed_actuals() {
        use crate::compiler::memory_layout::domain::compact::closed_engine_fixture;
        let (engine, budget, plan, _) = closed_engine_fixture(
            "use core::marker::PhantomData;fn marker<T>()->PhantomData<T>{PhantomData::<T>}fn main(){let _m=marker::<i32>();let _a=std::mem::align_of::<u16>();}",
            "main",
            Arc::from([]),
        );
        let declaration = (0..plan.callable_count()).map(|index| plan.callable_decl(index).unwrap()).find(|id| matches!(plan.persistent_decl_path(*id).unwrap().item(), frontend::PersistentDeclItem::Name(name) if name == "marker")).unwrap();
        let frontend::PublishedCallableBody::Source(publication) =
            plan.publish_callable_body(declaration, &budget).unwrap()
        else {
            panic!("original marker source required")
        };
        let template = cfg::lower_source_template(&publication, &budget).unwrap();
        let original = template.function().as_ref().clone();
        original
            .validate_source_publication(&publication, &budget)
            .unwrap();
        let target = ir::CallTarget {
            declaration,
            type_args: Arc::from([ir::Ty::Int(ir::IntTy::I32)]),
        };
        let mut specialization = opaque_specialization::Specialization::new(
            &publication,
            &target,
            &plan,
            &budget,
            engine.authority(),
        )
        .unwrap();
        let markers = |function: &cfg::Function| {
            let mut result = Vec::new();
            for block in &function.blocks {
                for statement in &block.statements {
                    statement
                        .value
                        .visit_operands(&mut |operand| {
                            if let cfg::Operand::Constant(cfg::Constant::PhantomData(marker)) =
                                operand
                            {
                                result.push(marker.clone());
                            }
                            Ok(())
                        })
                        .unwrap();
                }
                block
                    .terminator
                    .value
                    .visit_operands(&mut |operand| {
                        if let cfg::Operand::Constant(cfg::Constant::PhantomData(marker)) = operand
                        {
                            result.push(marker.clone());
                        }
                        Ok(())
                    })
                    .unwrap();
            }
            result
        };
        let source_markers = markers(&original);
        assert!(!source_markers.is_empty());
        assert!(
            source_markers
                .iter()
                .all(|marker| matches!(marker.argument(), ir::Ty::Param(_)))
        );
        let mut physical = original.clone();
        specialize_marker_constants(&mut physical, &mut specialization, &budget).unwrap();
        let actual_markers = markers(&physical);
        assert_eq!(actual_markers.len(), source_markers.len());
        let closed = plan.instance_facts(&target).unwrap();
        for (source, actual) in source_markers.iter().zip(&actual_markers) {
            assert_eq!(source.declaration(), actual.declaration());
            assert_eq!(actual.argument(), &ir::Ty::Int(ir::IntTy::I32));
            closed
                .validate_ty_with_budget(&ir::Ty::PhantomData(actual.clone()), &budget)
                .unwrap();
            let expected = specialization
                .ty(&ir::Ty::PhantomData(source.clone()))
                .unwrap();
            assert!(
                ir::identical_type(&expected, &ir::Ty::PhantomData(actual.clone()), &budget)
                    .unwrap()
            );
        }
        // The universal source and original operand proof stay unchanged.
        assert_eq!(markers(&original), source_markers);
        original
            .validate_source_publication(&publication, &budget)
            .unwrap();
        let changed =
            ir::PhantomDataTy::new(closed.world().canonical_catalog(), ir::Ty::Bool, &budget)
                .unwrap();
        let foreign_target = ir::CallTarget {
            declaration,
            type_args: Arc::from([ir::Ty::Bool]),
        };
        let mut foreign_specialization = opaque_specialization::Specialization::new(
            &publication,
            &foreign_target,
            &plan,
            &budget,
            engine.authority(),
        )
        .unwrap();
        let mut other = original.clone();
        specialize_marker_constants(&mut other, &mut foreign_specialization, &budget).unwrap();
        assert_eq!(markers(&other), vec![changed.clone(); source_markers.len()]);
        assert!(
            !ir::identical_type(
                &ir::Ty::PhantomData(actual_markers[0].clone()),
                &ir::Ty::PhantomData(changed.clone()),
                &budget
            )
            .unwrap()
        );
        let mut forged_source = original.clone();
        for block in &mut forged_source.blocks {
            for statement in &mut block.statements {
                if let cfg::Statement::Assign {
                    value:
                        cfg::Rvalue::Use(cfg::Operand::Constant(cfg::Constant::PhantomData(marker))),
                    ..
                } = &mut statement.value
                {
                    *marker = changed.clone();
                }
            }
        }
        assert!(
            forged_source
                .validate_source_publication(&publication, &budget)
                .is_err()
        );
        let denied = ir::PlanBudget::new(ir::PlanLimits {
            work: 0,
            ..Default::default()
        });
        assert!(
            specialize_marker_constants(&mut original.clone(), &mut specialization, &denied)
                .is_err()
        );
        assert!(denied.first_resource_failure().is_some());
    }

    #[test]
    fn closed_capture_family_refuses_fixed_and_nested_reference_carriers() {
        let budget = ir::PlanBudget::default();
        require_empty_physical_capture(&ir::Ty::Int(ir::IntTy::I32), &budget).unwrap();
        let reference = ir::Ty::Ref {
            kind: ir::BorrowKind::Shared,
            referent: Arc::new(ir::Ty::Int(ir::IntTy::I32)),
        };
        for actual in [
            reference.clone(),
            ir::Ty::Tuple(Arc::from([ir::Ty::Bool, reference.clone()])),
            ir::Ty::Array {
                element: Arc::new(reference),
                len: 2,
            },
        ] {
            assert!(require_empty_physical_capture(&actual, &budget).is_err());
        }
    }
}
