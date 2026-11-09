//! Exact carrier coordinates for graph-realized call signatures. The private
//! capsule is issued only from validated CFG provenance, not a caller-provided
//! type equivalence assertion. No source or executable authority is invented.
use super::*;
use crate::compiler::cfg;

mod storage;

#[derive(Clone, Debug)]
pub(super) struct NominalCarrierAuthority {
    canonical: Arc<CarrierContractView>,
    target: Option<Callee>,
    signature: Signature,
}

fn signature_copy(signature: &Signature, budget: &PlanBudget) -> Result<Signature> {
    let mut params = Vec::new();
    for parameter in &signature.params {
        push(budget, &mut params, parameter.clone())?;
    }
    Ok(Signature {
        ref_contract: signature.ref_contract.clone(),
        params,
        result: signature.result.clone(),
    })
}

/// Local construction cursor. Its immutable borrows prevent changing the CFG
/// or caller facts after provenance is checked. It is never a published fact.
pub(in crate::compiler) struct NominalCarrierSource<'a> {
    function: &'a cfg::Function,
    facts: &'a ValidatedFacts,
    validated: bool,
    // Local to one immutable CFG/facts analysis, never retained in a fact.
    // Existing proof caches authenticate complete source/current contexts on
    // every replay. Borrowed scratch cannot be reentered or shared globally.
    scratch: TraitScratch,
}

impl<'a> NominalCarrierSource<'a> {
    pub(in crate::compiler) fn into_scratch(self) -> TraitScratch {
        self.scratch
    }

    #[cfg(test)]
    pub(in crate::compiler) fn new(function: &'a cfg::Function, facts: &'a ValidatedFacts) -> Self {
        Self::with_scratch(function, facts, TraitScratch::default())
    }

    pub(in crate::compiler) fn with_scratch(
        function: &'a cfg::Function,
        facts: &'a ValidatedFacts,
        scratch: TraitScratch,
    ) -> Self {
        Self {
            function,
            facts,
            validated: false,
            scratch,
        }
    }

    pub(in crate::compiler) fn body(&mut self) -> Result<Arc<CarrierContractView>> {
        let facts = self.facts;
        let canonical = facts
            .source_carrier_contract_with_scratch(SignatureSubject::Body, &mut self.scratch)?;
        let mut params = Vec::new();
        for id in &self.function.params {
            push(
                facts.budget(),
                &mut params,
                self.function.locals[id.0].ty.clone(),
            )?;
        }
        let signature = Signature {
            params,
            result: self.function.result.clone(),
            ref_contract: self.function.ref_contract.clone(),
        };
        if canonical.source_schema.is_some() {
            facts.validate_source_flow_signature_with_scratch(
                SignatureSubject::Body,
                &signature,
                &mut self.scratch,
            )?;
        } else {
            facts.validate_signature_with(SignatureSubject::Body, &signature, &mut self.scratch)?;
        }
        let mut different = false;
        for (before, after) in canonical
            .result_signature
            .params
            .iter()
            .zip(&signature.params)
            .chain(std::iter::once((
                &canonical.result_signature.result,
                &signature.result,
            )))
        {
            different |=
                charged(
                    facts.budget(),
                    ir::carrier_locations(before, facts.budget()),
                )? != charged(facts.budget(), ir::carrier_locations(after, facts.budget()))?;
        }
        if !different {
            return Ok(canonical);
        }
        if self.function.lowering_proof.is_none() {
            return Err(FactsError::invalid(
                facts.budget(),
                "graph body carrier transport requires exact source lowering proof",
            ));
        }
        if !self.validated {
            self.function
                .validate_provenance_with_scratch(facts, facts.budget(), &mut self.scratch)
                .map_err(|error| FactsError::from_message(facts.budget(), &error))?;
            self.validated = true;
        }
        retain::<NominalCarrierAuthority>(facts.budget(), 1)?;
        let authority = Arc::new(NominalCarrierAuthority {
            canonical,
            target: None,
            signature,
        });
        let mut view = authority.derive_with_scratch(facts, &mut self.scratch)?;
        view.nominal = Some(authority);
        view.validate_for_with_scratch(facts, &mut self.scratch)?;
        retain::<CarrierContractView>(facts.budget(), 1)?;
        Ok(Arc::new(view))
    }

    pub(in crate::compiler) fn call(
        &mut self,
        block: cfg::BlockId,
    ) -> Result<Arc<CarrierContractView>> {
        let function = self.function;
        let facts = self.facts;
        let Some(cfg::BasicBlock {
            terminator:
                cfg::Located {
                    value:
                        cfg::Terminator::Call {
                            target, signature, ..
                        },
                    ..
                },
            ..
        }) = function.blocks.get(block.0)
        else {
            return Err(FactsError::invalid(
                facts.budget(),
                "carrier source is not a call",
            ));
        };
        let canonical = facts.source_carrier_contract_with_scratch(
            SignatureSubject::Call(target),
            &mut self.scratch,
        )?;
        if canonical.source_schema.is_some() {
            facts.validate_source_flow_signature_with_scratch(
                SignatureSubject::Call(target),
                signature,
                &mut self.scratch,
            )?;
        } else {
            facts.validate_signature_with(
                SignatureSubject::Call(target),
                signature,
                &mut self.scratch,
            )?;
        }
        let mut different = false;
        for (before, after) in canonical
            .result_signature
            .params
            .iter()
            .zip(&signature.params)
            .chain(std::iter::once((
                &canonical.result_signature.result,
                &signature.result,
            )))
        {
            let before = charged(
                facts.budget(),
                ir::carrier_locations(before, facts.budget()),
            )?;
            let after = charged(facts.budget(), ir::carrier_locations(after, facts.budget()))?;
            different |= before != after;
        }
        if !different {
            return Ok(canonical);
        }
        if function.lowering_proof.is_none() {
            return Err(FactsError::invalid(
                facts.budget(),
                "graph carrier transport requires exact source lowering proof",
            ));
        }
        if !self.validated {
            function
                .validate_provenance_with_scratch(facts, facts.budget(), &mut self.scratch)
                .map_err(|error| FactsError::from_message(facts.budget(), &error))?;
            self.validated = true;
        }
        retain::<NominalCarrierAuthority>(facts.budget(), 1)?;
        let authority = Arc::new(NominalCarrierAuthority {
            canonical,
            target: Some(target.clone()),
            signature: signature_copy(signature, facts.budget())?,
        });
        let mut view = authority.derive_with_scratch(facts, &mut self.scratch)?;
        view.nominal = Some(authority);
        view.validate_for_with_scratch(facts, &mut self.scratch)?;
        retain::<CarrierContractView>(facts.budget(), 1)?;
        Ok(Arc::new(view))
    }
}

#[cfg(test)]
impl ValidatedFacts {
    fn cfg_call_carriers(
        &self,
        function: &cfg::Function,
        block: cfg::BlockId,
    ) -> Result<Arc<CarrierContractView>> {
        NominalCarrierSource::new(function, self).call(block)
    }
}

impl NominalCarrierAuthority {
    fn derive_with_scratch(
        &self,
        facts: &ValidatedFacts,
        scratch: &mut TraitScratch,
    ) -> Result<CarrierContractView> {
        let original = &self.canonical;
        let original_replay = original.validate_for_replay_with_scratch(facts, scratch)?;
        let subject = match &self.target {
            Some(Callee::Direct(target)) => {
                if !matches!(&original.result_subject, parameter_equality::EqualityResultSubject::Direct(actual) if actual == target)
                {
                    return Err(FactsError::invalid(
                        facts.budget(),
                        "graph carrier transport differs from its canonical target",
                    ));
                }
                SignatureSubject::Call(self.target.as_ref().unwrap())
            }
            None if matches!(
                &original.result_subject,
                parameter_equality::EqualityResultSubject::Body { .. }
            ) =>
            {
                SignatureSubject::Body
            }
            _ => {
                return Err(FactsError::invalid(
                    facts.budget(),
                    "graph carrier transport differs from its canonical subject",
                ));
            }
        };
        if original.nominal.is_some()
            || original.physical.is_some()
            || original.closure_result.is_some()
            || original.projected.is_some()
            || original.equality_required.is_some()
            || matches!(self.target, Some(ref target) if !matches!(target, Callee::Direct(_)))
        {
            return Err(FactsError::unsupported(
                facts.budget(),
                "graph carrier transport requires an ordinary direct declaration",
            ));
        }
        let source_schema = match &original.source_schema {
            Some(source) => Some(source.reissue_for_signature(
                original,
                subject,
                &self.signature,
                facts,
                original_replay.as_ref().ok_or_else(|| {
                    FactsError::invalid(
                        facts.budget(),
                        "nominal transport lost original source reconstruction",
                    )
                })?,
                scratch,
            )?),
            None => {
                facts.validate_signature(subject, &self.signature)?;
                None
            }
        };
        let world = facts.world();
        let subject = world.equality_result_subject(subject, facts)?;
        let raw = world.normalized_declaration_schema(original.declaration)?;
        let retained = world.declaration_carriers(original.declaration)?;
        let storage_arguments = self.storage_arguments(facts)?;
        let mut view = world.transport_carrier_parts_with_scratch(
            original.declaration,
            storage_arguments,
            signature_copy(&self.signature, facts.budget())?,
            facts,
            raw,
            retained,
            original.origin.clone(),
            None,
            subject,
            scratch,
        )?;
        // Storage expansion uses the exact mapped parameter occurrences, but
        // never retags canonical source entry arguments or receipt identities.
        view.arguments = original.arguments.clone();
        view.source_schema = source_schema;
        view.executable_storage = original.executable_storage;
        // Restricted equality results are recomputed from the transported
        // subject, never copied from the canonical view; `validate_for`
        // rederives and compares them again.
        world.seal_equality_results_with_scratch(&mut view, facts, scratch)?;
        Ok(view)
    }

    pub(super) fn validate(
        &self,
        view: &CarrierContractView,
        facts: &ValidatedFacts,
    ) -> Result<()> {
        self.validate_with_scratch(view, facts, &mut TraitScratch::default())
    }

    pub(super) fn validate_with_scratch(
        &self,
        view: &CarrierContractView,
        facts: &ValidatedFacts,
        scratch: &mut TraitScratch,
    ) -> Result<()> {
        let expected = self.derive_with_scratch(facts, scratch)?;
        let same_source = match (&view.source_schema, &expected.source_schema) {
            (None, None) => true,
            (Some(actual), Some(expected)) => actual.same_authority(expected, facts.budget())?,
            _ => false,
        };
        if view.executable_storage != expected.executable_storage
            || !same_source
            || view.declaration != expected.declaration
            || view.arguments != expected.arguments
            || !facts
                .world()
                .same_env(&view.original_env, &expected.original_env)?
            || view.original_contract != expected.original_contract
            || view.contract != expected.contract
            || view.origin != expected.origin
            || view.fixed != expected.fixed
            || view.regions != expected.regions
            || view.source_regions != expected.source_regions
            || view.components != expected.components
            || view.nominals != expected.nominals
            || view.result_signature != expected.result_signature
            || !facts
                .world()
                .same_equality_result_subject(&view.result_subject, &expected.result_subject)?
            || view.closure_result.is_some()
            || view.projected.is_some()
            || view.projected_subject.is_some()
            || view.equality_required.is_some()
            || view.equality_results.is_some() != expected.equality_results.is_some()
        {
            return Err(FactsError::invalid(
                facts.budget(),
                "graph carrier view differs from its exact nominal transport",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::{self, Input, Snapshot, source};
    use crate::engine::Engine;

    fn graph_view(
        workers: usize,
    ) -> (
        Arc<cfg::Function>,
        Arc<ValidatedFacts>,
        cfg::BlockId,
        Arc<CarrierContractView>,
    ) {
        graph_subject(workers, false)
    }

    fn graph_subject(
        workers: usize,
        body: bool,
    ) -> (
        Arc<cfg::Function>,
        Arc<ValidatedFacts>,
        cfg::BlockId,
        Arc<CarrierContractView>,
    ) {
        graph_subject_source(workers, body, None)
    }

    fn graph_subject_source(
        workers: usize,
        body: bool,
        sealed_source: Option<&str>,
    ) -> (
        Arc<cfg::Function>,
        Arc<ValidatedFacts>,
        cfg::BlockId,
        Arc<CarrierContractView>,
    ) {
        let snapshot = Arc::new(Snapshot {
            input: Input::Text(if let Some(source) = sealed_source {
                source
            } else if body {
                "fn split(s:&[i32])->(Option<&[i32]>,&[i32]){(Some(&s[..2]),&s[2..])} fn main(){let a=[1i32,2,3];let(x,y)=split(&a);println!(\"{} {}\",x.unwrap().len(),y.len());}"
            } else {
                "struct W<'a,T>{r:&'a T} impl<'a,T> W<'a,T>{fn next(&mut self)->Option<&'a T>{Some(self.r)}} fn main(){let n=7i32;let mut w=W{r:&n};let x=w.next();match x{Some(r)=>println!(\"{}\",*r),None=>()}}"
            }.into()),
            source_options: source::SourceOptions::default(),
            budget: PlanBudget::default(),
        });
        let engine = Engine::new(compiler::CompilerProvider(snapshot));
        let check = engine.request(crate::compiler::keys::CheckRoot.key());
        let report = engine.run(workers).unwrap();
        check.completion().unwrap();
        assert!(report.capture_complete);
        assert!(!report.node_evaluations.iter().any(|(key, _)| matches!(
            key.kind,
            compiler::BUILD | compiler::OBJECT | compiler::INSTANCE
        )));
        for (key, _) in &report.node_evaluations {
            let (function, facts) = match key.kind {
                compiler::TEMPLATE_BODY => {
                    // Closed source admission lowers its immutable publication
                    // inside the admission node rather than publishing LOWER.
                    let body = engine
                        .fact_of::<compiler::TemplateBodyFact>(key, 0)
                        .unwrap();
                    let Some(publication) = &body.publication else {
                        continue;
                    };
                    if publication.facts().phase() != FactsPhase::Closed {
                        continue;
                    }
                    let function =
                        cfg::lower_source_closed(publication, publication.facts().budget())
                            .unwrap();
                    (Arc::new(function), publication.facts().clone())
                }
                compiler::UNIVERSAL_LOWER => {
                    let lowered = engine.fact_of::<cfg::TemplateCfg>(key, 0).unwrap();
                    (lowered.function().clone(), lowered.facts().clone())
                }
                compiler::LOWER => {
                    let function = engine.fact_of::<Arc<cfg::Function>>(key, 0).unwrap();
                    let body = engine
                        .fact_of::<compiler::InstanceBodyFact>(
                            &crate::compiler::keys::words(compiler::BODY, key.params.clone())
                                .unwrap(),
                            0,
                        )
                        .unwrap();
                    (function.as_ref().clone(), body.facts.clone())
                }
                _ => continue,
            };
            if body {
                let view = NominalCarrierSource::new(&function, &facts).body().unwrap();
                if view.nominal.is_some() {
                    return (function, facts, cfg::BlockId(0), view);
                }
                continue;
            }
            for (index, block) in function.blocks.iter().enumerate() {
                if !matches!(block.terminator.value, cfg::Terminator::Call { .. }) {
                    continue;
                }
                let block = cfg::BlockId(index);
                let view = facts.cfg_call_carriers(&function, block).unwrap();
                if let Some(nominal) = &view.nominal {
                    if sealed_source.is_some() {
                        let original = nominal.canonical.source_schema.as_ref().unwrap();
                        let transported = view.source_schema.as_ref().unwrap();
                        if original
                            .same_authority(transported, facts.budget())
                            .unwrap()
                        {
                            continue;
                        }
                    }
                    return (function, facts, block, view);
                }
            }
        }
        panic!("CHECK did not publish the expected graph nominal call");
    }

    #[test]
    fn graph_source_carrier_transport_reissues_exact_sealed_inputs() {
        for workers in [1, 8] {
            {
                let (_, facts, _, view) = graph_subject_source(
                    workers,
                    false,
                    Some(
                        "fn main(){let mut calls=0i32;let mut it=(0i32..2i32).flat_map(|n|{calls+=1;0i32..n});let _=it.next();}",
                    ),
                );
                let authority = view.nominal.as_ref().unwrap();
                assert_eq!(view.arguments, authority.canonical.arguments);
                let mut scratch = TraitScratch::default();
                for _ in 0..3 {
                    view.validate_for_with_scratch(&facts, &mut scratch)
                        .unwrap();
                    for input in view.contract.inputs.iter() {
                        ir::carrier_at(
                            &view.result_signature.params[input.param],
                            &input.location,
                            facts.budget(),
                        )
                        .unwrap();
                    }
                    for parameter in view.original_env.ordered_params() {
                        if let Some(Ty::Struct(actual)) = view
                            .family_actual_with_scratch(
                                &ir::CarrierFamily::Parameter(parameter),
                                &facts,
                                &mut scratch,
                            )
                            .unwrap()
                        {
                            assert!(actual.id().index.is_graph());
                        }
                    }
                }
                let family = ir::CarrierFamily::Parameter(
                    view.original_env.ordered_params().next().unwrap(),
                );
                let replay = view.replay_for_families(&facts, &mut scratch).unwrap();
                let before = facts.budget().usage_snapshot().work;
                let expected = view
                    .family_actual_with_scratch(&family, &facts, &mut scratch)
                    .unwrap();
                let repeated = facts.budget().usage_snapshot().work - before;
                let before = facts.budget().usage_snapshot().work;
                for _ in 0..3 {
                    assert_eq!(
                        view.family_actual_with_replay(&family, &facts, &replay, &mut scratch)
                            .unwrap(),
                        expected
                    );
                }
                let borrowed = facts.budget().usage_snapshot().work - before;
                assert!(
                    borrowed < repeated.saturating_mul(3),
                    "family consumers reconstructed full view: {borrowed}/{repeated}"
                );
                let detached = (*view).clone();
                assert!(
                    detached
                        .family_actual_with_replay(&family, &facts, &replay, &mut scratch)
                        .is_err()
                );
                let mut changed_stage = detached.clone();
                changed_stage.phase = FactsPhase::Schema;
                assert!(
                    changed_stage
                        .family_actual_with_replay(&family, &facts, &replay, &mut scratch)
                        .is_err()
                );
                let (_, foreign, _, _) = graph_view(workers);
                assert!(
                    view.family_actual_with_replay(&family, &foreign, &replay, &mut scratch)
                        .is_err()
                );
                let mut forged = (*view).clone();
                let mut inputs = forged.contract.inputs.to_vec();
                assert!(!inputs.is_empty());
                let mut path = inputs[0].location.path.0.to_vec();
                path.push(ir::CarrierStep::Field(usize::MAX));
                inputs[0].location.path = ir::CarrierPath(path.into());
                forged.contract.inputs = inputs.into();
                assert!(
                    forged
                        .validate_for_with_scratch(&facts, &mut scratch)
                        .is_err()
                );
                assert!(
                    forged
                        .family_actual_with_replay(&family, &facts, &replay, &mut scratch)
                        .is_err()
                );
                assert_eq!(
                    view.family_actual_with_replay(&family, &facts, &replay, &mut scratch)
                        .unwrap(),
                    expected
                );
                facts.budget().poison_ledger_for_tests();
                assert!(
                    matches!(view.family_actual_with_replay(&family, &facts, &replay, &mut scratch), Err(error) if error.is_resource())
                );
            }
            let (_, facts, _, view) = graph_subject_source(
                workers,
                false,
                Some("fn main(){let mut it=(0i32..2i32).flat_map(|n|0i32..n);let _=it.next();}"),
            );
            let nominal = view.nominal.as_ref().unwrap();
            let original = nominal.canonical.source_schema.as_ref().unwrap();
            let transported = view.source_schema.as_ref().unwrap();
            assert!(!Arc::ptr_eq(original, transported));
            assert!(
                !original
                    .same_authority(transported, facts.budget())
                    .unwrap()
            );
            let mut scratch = TraitScratch::default();
            let canonical = &nominal.canonical;
            let canonical_replay = canonical
                .validate_for_replay_with_scratch(&facts, &mut scratch)
                .unwrap()
                .unwrap();
            let subject = SignatureSubject::Call(nominal.target.as_ref().unwrap());
            for _ in 0..3 {
                let reissued = original
                    .reissue_for_signature(
                        canonical,
                        subject,
                        &view.result_signature,
                        &facts,
                        &canonical_replay,
                        &mut scratch,
                    )
                    .unwrap();
                assert!(
                    reissued
                        .same_authority(transported, facts.budget())
                        .unwrap()
                );
            }
            let detached_canonical = (**canonical).clone();
            assert!(
                original
                    .reissue_for_signature(
                        &detached_canonical,
                        subject,
                        &view.result_signature,
                        &facts,
                        &canonical_replay,
                        &mut scratch,
                    )
                    .is_err()
            );
            let mut changed_canonical = (**canonical).clone();
            changed_canonical.phase = FactsPhase::Schema;
            assert!(
                original
                    .reissue_for_signature(
                        &changed_canonical,
                        subject,
                        &view.result_signature,
                        &facts,
                        &canonical_replay,
                        &mut scratch,
                    )
                    .is_err()
            );
            assert!(
                transported
                    .reissue_for_signature(
                        canonical,
                        subject,
                        &view.result_signature,
                        &facts,
                        &canonical_replay,
                        &mut scratch,
                    )
                    .is_err()
            );
            let mut changed_signature = view.result_signature.clone();
            changed_signature.result = Ty::Bool;
            assert!(
                original
                    .reissue_for_signature(
                        canonical,
                        subject,
                        &changed_signature,
                        &facts,
                        &canonical_replay,
                        &mut scratch,
                    )
                    .is_err()
            );
            view.validate_for_with_scratch(&facts, &mut scratch)
                .unwrap();
            view.validate_for_with_scratch(&facts, &mut scratch)
                .unwrap();
            let source_replay = transported
                .replay_with_scratch(&view, &facts, &mut scratch)
                .unwrap();
            for _ in 0..3 {
                facts
                    .world
                    .validate_equality_results_with_source_replay(
                        &view,
                        &facts,
                        Some(&source_replay),
                        &mut scratch,
                    )
                    .unwrap();
            }
            assert!(facts.world.validate_equality_results_with_source_replay(
                &view, &facts, None, &mut scratch,
            ).is_err());
            let detached_view = (*view).clone();
            assert!(
                facts
                    .world
                    .validate_equality_results_with_source_replay(
                        &detached_view,
                        &facts,
                        Some(&source_replay),
                        &mut scratch,
                    )
                    .is_err()
            );
            let mut changed_stage = (*view).clone();
            changed_stage.phase = FactsPhase::Schema;
            assert!(
                facts
                    .world
                    .validate_equality_results_with_source_replay(
                        &changed_stage,
                        &facts,
                        Some(&source_replay),
                        &mut scratch,
                    )
                    .is_err()
            );
            let operations = nominal_normalization::NominalOperations::default();
            let replay_roots = || -> Result<()> {
                for ty in view
                    .result_signature
                    .params
                    .iter()
                    .chain(std::iter::once(&view.result_signature.result))
                {
                    facts.world.map_universal_projection_type_in(
                        ty,
                        facts.env(),
                        0,
                        Some((&facts, transported.transport())),
                        false,
                        &operations,
                        &mut |projection| Ok(Ty::AssociatedType(Arc::new(projection.clone()))),
                    )?;
                }
                Ok(())
            };
            let before = facts.budget().usage_snapshot().work;
            replay_roots().unwrap();
            let cold = facts.budget().usage_snapshot().work - before;
            let before = facts.budget().usage_snapshot().work;
            replay_roots().unwrap();
            let warm = facts.budget().usage_snapshot().work - before;
            assert!(
                warm < cold,
                "whole signature mapper discarded exact transport arena: {warm}/{cold}"
            );
            let mut rows = nominal_normalization::ParentLocalValidation::default();
            let before = facts.budget().usage_snapshot().work;
            transported
                .transport()
                .validate_with_replay(&facts.world, &facts, &mut rows)
                .unwrap();
            let row_cold = facts.budget().usage_snapshot().work - before;
            ParentLocalTransport::default()
                .validate_with_replay(&facts.world, &facts, &mut rows)
                .unwrap();
            let mut repeated = transported.transport().definitions().to_vec();
            assert!(!repeated.is_empty());
            repeated.push(repeated[0].clone());
            ParentLocalTransport::new(&repeated)
                .validate_with_replay(&facts.world, &facts, &mut rows)
                .unwrap();
            let before = facts.budget().usage_snapshot().work;
            transported
                .transport()
                .validate_with_replay(&facts.world, &facts, &mut rows)
                .unwrap();
            let row_warm = facts.budget().usage_snapshot().work - before;
            assert!(
                row_warm < row_cold,
                "sibling invocation evicted row DAG: {row_warm}/{row_cold}"
            );
            let context = ProofContext {
                transport: transported.transport(),
                ..ProofContext::from_facts(&facts)
            };
            let (declaration, arguments, _) = facts
                .world
                .source_carrier_application(
                    &view.result_subject,
                    &facts,
                    context,
                    scratch.for_world(facts.world.id),
                )
                .unwrap();
            let schema = match &view.result_subject {
                parameter_equality::EqualityResultSubject::Required(call) => {
                    facts.world.item_header(call.item).unwrap().schema.clone()
                }
                _ => facts.world.callable_schema(declaration).unwrap(),
            };
            let mut substitutions = schema_instantiation::Substitutions::default();
            let before = facts.budget().usage_snapshot().work;
            facts
                .world
                .instantiate_raw_cached(
                    &schema,
                    &arguments,
                    context,
                    Some(declaration),
                    &mut substitutions,
                )
                .unwrap();
            let substitution_cold = facts.budget().usage_snapshot().work - before;
            let before = facts.budget().usage_snapshot().work;
            facts
                .world
                .instantiate_raw_cached(
                    &schema,
                    &arguments,
                    context,
                    Some(declaration),
                    &mut substitutions,
                )
                .unwrap();
            let substitution_warm = facts.budget().usage_snapshot().work - before;
            assert!(
                substitution_warm < substitution_cold,
                "raw substitution discarded sealed replay DAG: {substitution_warm}/{substitution_cold}"
            );
            let mut transplanted = view.as_ref().clone();
            transplanted.source_schema = Some(original.clone());
            assert!(
                transplanted
                    .validate_for_with_scratch(&facts, &mut scratch)
                    .is_err()
            );
            let mut changed = view.as_ref().clone();
            changed.result_signature.result = Ty::Bool;
            assert!(
                changed
                    .validate_for_with_scratch(&facts, &mut scratch)
                    .is_err()
            );
            let (_, foreign, _, _) = graph_view(workers);
            assert!(
                original
                    .reissue_for_signature(
                        canonical,
                        subject,
                        &view.result_signature,
                        &foreign,
                        &canonical_replay,
                        &mut scratch,
                    )
                    .is_err()
            );
            assert!(
                facts
                    .world
                    .validate_equality_results_with_source_replay(
                        &view,
                        &foreign,
                        Some(&source_replay),
                        &mut scratch,
                    )
                    .is_err()
            );
            assert!(
                transported
                    .transport()
                    .validate_with_replay(&foreign.world, &foreign, &mut rows)
                    .is_err()
            );
            assert!(
                facts
                    .world
                    .instantiate_raw_cached(
                        &schema,
                        &arguments,
                        ProofContext {
                            transport: transported.transport(),
                            ..ProofContext::from_facts(&foreign)
                        },
                        Some(declaration),
                        &mut substitutions
                    )
                    .is_err()
            );
            assert!(
                foreign
                    .world
                    .map_universal_projection_type_in(
                        &Ty::Unit,
                        foreign.env(),
                        0,
                        Some((&foreign, transported.transport())),
                        false,
                        &operations,
                        &mut |projection| Ok(Ty::AssociatedType(Arc::new(projection.clone()))),
                    )
                    .is_err()
            );
            assert!(
                view.validate_for_with_scratch(&foreign, &mut scratch)
                    .is_err()
            );
            view.validate_for_with_scratch(&facts, &mut scratch)
                .unwrap();
            facts.budget().poison_ledger_for_tests();
            assert!(matches!(original.reissue_for_signature(
                canonical,
                subject,
                &view.result_signature,
                &facts,
                &canonical_replay,
                &mut scratch,
            ), Err(error) if error.is_resource()));
            assert!(
                matches!(facts.world.validate_equality_results_with_source_replay(
                &view, &facts, Some(&source_replay), &mut scratch,
            ), Err(error) if error.is_resource())
            );
            assert!(
                matches!(transported.transport().validate_with_replay(&facts.world, &facts, &mut rows), Err(error) if error.is_resource())
            );
            assert!(
                matches!(facts.world.instantiate_raw_cached(&schema, &arguments, context, Some(declaration), &mut substitutions), Err(error) if error.is_resource())
            );
            assert!(matches!(replay_roots(), Err(error) if error.is_resource()));
            assert!(
                matches!(view.validate_for_with_scratch(&facts, &mut scratch), Err(error) if error.is_resource())
            );
        }
    }

    #[test]
    fn storage_wrapper_occurrences_preserve_ordered_children_and_identity() {
        use crate::compiler::frontend::{self, PlanCtx};
        let program = frontend::parse("fn id<T>(x:T)->T{x}fn main(){}").unwrap();
        let budget = PlanBudget::default();
        let plan = PlanCtx::from_program(&program, &budget).unwrap();
        let world = plan.trait_world().unwrap();
        let declaration = plan.function_decl(0).unwrap();
        let other = plan.function_decl(1).unwrap();
        let schema = world.callable_schema(declaration).unwrap();
        let parameter = Ty::Param(schema.env.ordered_params().next().unwrap());
        let foreign_plan = PlanCtx::from_program(&program, &budget).unwrap();
        let foreign_world = foreign_plan.trait_world().unwrap();
        let make = |kind, ty: Ty, foreign| {
            let identity = if foreign { other } else { declaration };
            let catalog = if foreign {
                &foreign_world.catalog
            } else {
                &world.catalog
            };
            match kind {
                0 => Ty::PhantomData(ir::PhantomDataTy::new(catalog, ty, &budget).unwrap()),
                1 => Ty::ManuallyDrop(ir::ManuallyDropTy::new(catalog, ty, &budget).unwrap()),
                2 => Ty::FnPointer(Arc::new(ir::FnPointerTy {
                    inputs: if foreign {
                        Arc::from([])
                    } else {
                        Arc::from([ty.clone()])
                    },
                    output: ty,
                })),
                3 => Ty::FnItem(Arc::new(CallTarget {
                    declaration: identity,
                    type_args: Arc::from([ty]),
                })),
                4 => Ty::Closure(Arc::new(ir::ClosureTy::new(
                    declaration,
                    Arc::from([ty.clone()]),
                    0,
                    identity,
                    Arc::from([ty.clone()]),
                    ty.clone(),
                    Arc::from([ty]),
                    ir::ClosureKind::Fn,
                ))),
                5 => Ty::Opaque(Arc::new(ir::OpaqueTy::new(
                    identity,
                    Arc::from([ty.clone()]),
                    0,
                    Arc::from([ir::OpaqueBound::new(
                        declaration,
                        Arc::from([ir::OpaqueBoundArg::Type(ty)]),
                        Arc::from([]),
                    )]),
                    Arc::from([]),
                ))),
                _ => Ty::Dyn(Arc::new(ir::DynTy {
                    principal: identity,
                    args: Arc::from([ty.clone()]),
                    associated_types: Arc::from([(declaration, ty)]),
                })),
            }
        };
        for kind in 0..7 {
            let original = make(kind, parameter.clone(), false);
            let mapped = make(kind, Ty::Bool, false);
            let mut found = [None];
            storage::storage_occurrences(
                &world,
                &schema.env,
                &[Ty::Bool],
                &original,
                &mapped,
                &mut found,
                &mut Vec::new(),
                0,
            )
            .unwrap();
            assert_eq!(
                found,
                [Some(Ty::Bool)],
                "wrapper {kind} lost its semantic child"
            );
            for rejected in [
                make(kind, Ty::Bool, true),
                make(kind, Ty::Int(ir::IntTy::U8), false),
            ] {
                assert!(
                    storage::storage_occurrences(
                        &world,
                        &schema.env,
                        &[Ty::Bool],
                        &original,
                        &rejected,
                        &mut [None],
                        &mut Vec::new(),
                        0,
                    )
                    .is_err(),
                    "wrapper {kind} accepted changed identity or child"
                );
            }
        }
        assert!(
            storage::storage_occurrences(
                &world,
                &schema.env,
                &[Ty::Bool],
                &Ty::Ref {
                    kind: ir::BorrowKind::Shared,
                    referent: Arc::new(parameter)
                },
                &Ty::Bool,
                &mut [None],
                &mut Vec::new(),
                0,
            )
            .is_err()
        );
        budget.poison_ledger_for_tests();
        assert!(matches!(storage::storage_occurrences(
            &world, &schema.env, &[Ty::Bool], &Ty::Bool, &Ty::Bool,
            &mut [None], &mut Vec::new(), 0,
        ), Err(error) if error.is_resource()));
    }

    #[test]
    fn graph_body_carriers_reject_forged_authority() {
        for workers in [1, 8] {
            let (function, facts, _, view) = graph_subject(workers, true);
            view.validate_for(&facts).unwrap();
            let (_, foreign, _, _) = graph_subject(workers, true);
            assert!(view.validate_for(&foreign).is_err());
            let mut detached = (*function).clone();
            detached.lowering_proof = None;
            assert!(NominalCarrierSource::new(&detached, &facts).body().is_err());
            let mut forged = (*function).clone();
            forged.result = Ty::Bool;
            assert!(NominalCarrierSource::new(&forged, &facts).body().is_err());
            let mut changed = (*view).clone();
            Arc::make_mut(changed.nominal.as_mut().unwrap())
                .signature
                .result = Ty::Bool;
            assert!(changed.validate_for(&facts).is_err());
            let (_, _, _, call) = graph_view(workers);
            let mut changed = (*view).clone();
            Arc::make_mut(changed.nominal.as_mut().unwrap()).target =
                call.nominal.as_ref().unwrap().target.clone();
            assert!(changed.validate_for(&facts).is_err());
            let mut changed = (*view).clone();
            changed.contract.results = Arc::from([]);
            assert!(changed.validate_for(&facts).is_err());
        }
    }

    #[test]
    fn graph_nominal_carrier_views_reject_forged_authority() {
        for workers in [1, 8] {
            let (function, facts, block, view) = graph_view(workers);
            view.validate_for(&facts).unwrap();
            let (_, foreign, _, _) = graph_view(workers);
            assert!(view.validate_for(&foreign).is_err());
            let mut changed = (*view).clone();
            changed.phase = FactsPhase::Schema;
            assert!(changed.validate_for(&facts).is_err());
            let mut changed = (*view).clone();
            changed.nominals = Arc::from([]);
            assert!(changed.validate_for(&facts).is_err());
            let mut changed = (*view).clone();
            let mut components = changed.components.to_vec();
            components.push(ir::CarrierComponentMapping {
                original_family: ir::CarrierFamily::Parameter(ParamId {
                    owner: ir::BinderId {
                        declaration: changed.declaration,
                    },
                    index: 0,
                }),
                original: changed.contract.results[0].location.clone(),
                substituted: changed.contract.results[0].location.clone(),
            });
            changed.components = components.into();
            assert!(changed.validate_for(&facts).is_err());
            let mut changed = (*view).clone();
            changed.contract.results = Arc::from([]);
            assert!(changed.validate_for(&facts).is_err());
            let mut changed = (*view).clone();
            Arc::make_mut(&mut changed.origin).result_tails = Arc::from([ir::ResultCarrierTail {
                prefix: 0,
                parameter: ParamId {
                    owner: ir::BinderId {
                        declaration: changed.declaration,
                    },
                    index: 0,
                },
                inputs: Arc::from([]),
            }]);
            assert!(changed.validate_for(&facts).is_err());
            let mut changed = (*view).clone();
            Arc::make_mut(changed.nominal.as_mut().unwrap())
                .signature
                .result = Ty::Bool;
            assert!(changed.validate_for(&facts).is_err());
            let mut changed = (*view).clone();
            let authority = Arc::make_mut(changed.nominal.as_mut().unwrap());
            let Some(Callee::Direct(target)) = &mut authority.target else {
                unreachable!()
            };
            target.declaration = facts.declaration();
            assert!(changed.validate_for(&facts).is_err());
            let mut changed = (*view).clone();
            let authority = Arc::make_mut(changed.nominal.as_mut().unwrap());
            Arc::make_mut(&mut authority.canonical).nominal = view.nominal.clone();
            assert!(changed.validate_for(&facts).is_err());
            let mut changed = (*view).clone();
            changed.projected_subject = Some(TraitRef {
                declaration: changed.declaration,
                self_ty: Ty::Bool,
                args: Arc::from([]),
            });
            assert!(changed.validate_for(&facts).is_err());
            let mut changed = (*view).clone();
            changed.closure_result = Some((
                Arc::new(ir::ClosureTy::new(
                    changed.declaration,
                    Arc::from([]),
                    0,
                    changed.declaration,
                    Arc::from([]),
                    Ty::Bool,
                    Arc::from([]),
                    ir::ClosureKind::Fn,
                )),
                ir::ClosureKind::Fn,
            ));
            assert!(changed.validate_for(&facts).is_err());
            let program = compiler::frontend::parse("trait Source:Sized{type Out;fn get(self)->Self::Out;} fn produce<T:Source<Out=U>,U>(x:T)->U{x.get()} fn main(){}").unwrap();
            let plan = compiler::frontend::PlanCtx::from_program(&program, &PlanBudget::default())
                .unwrap();
            let equality_world = plan.trait_world().unwrap();
            let declaration = equality_world
                .headers
                .callables
                .iter()
                .find(|header| header.schema.env.total_arity() == 2)
                .unwrap()
                .declaration;
            let equality_facts = equality_world.callable_facts(declaration).unwrap();
            let equality_view = equality_facts
                .carrier_contract(SignatureSubject::Body)
                .unwrap();
            assert!(equality_view.equality_results.is_some());
            let mut changed = (*view).clone();
            changed.equality_results = equality_view.equality_results.clone();
            assert!(changed.validate_for(&facts).is_err());
            let mut detached = (*function).clone();
            detached.lowering_proof = None;
            assert!(facts.cfg_call_carriers(&detached, block).is_err());
            let mut forged = (*function).clone();
            let cfg::Terminator::Call { signature, .. } =
                &mut forged.blocks[block.0].terminator.value
            else {
                unreachable!()
            };
            signature.result = Ty::Bool;
            assert!(facts.cfg_call_carriers(&forged, block).is_err());
            let (other_function, _, _, _) = graph_view(workers);
            let mut transplanted = (*function).clone();
            transplanted.lowering_proof = other_function.lowering_proof.clone();
            assert!(facts.cfg_call_carriers(&transplanted, block).is_err());
        }
    }
}
