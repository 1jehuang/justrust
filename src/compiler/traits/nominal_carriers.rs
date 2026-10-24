//! Exact carrier coordinates for graph-realized call signatures. The private
//! capsule is issued only from validated CFG provenance, not a caller-provided
//! type equivalence assertion. No source or executable authority is invented.
use super::*;
use crate::compiler::cfg;

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
}

impl<'a> NominalCarrierSource<'a> {
    pub(in crate::compiler) fn new(function: &'a cfg::Function, facts: &'a ValidatedFacts) -> Self {
        Self {
            function,
            facts,
            validated: false,
        }
    }

    pub(in crate::compiler) fn body(&mut self) -> Result<Arc<CarrierContractView>> {
        let facts = self.facts;
        let canonical = facts.carrier_contract(SignatureSubject::Body)?;
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
        facts.validate_signature(SignatureSubject::Body, &signature)?;
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
                .validate_provenance(facts, facts.budget())
                .map_err(|error| FactsError::from_message(facts.budget(), &error))?;
            self.validated = true;
        }
        retain::<NominalCarrierAuthority>(facts.budget(), 1)?;
        let authority = Arc::new(NominalCarrierAuthority {
            canonical,
            target: None,
            signature,
        });
        let mut view = authority.derive(facts)?;
        view.nominal = Some(authority);
        view.validate_for(facts)?;
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
        let canonical = facts.carrier_contract(SignatureSubject::Call(target))?;
        facts.validate_signature(SignatureSubject::Call(target), signature)?;
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
                .validate_provenance(facts, facts.budget())
                .map_err(|error| FactsError::from_message(facts.budget(), &error))?;
            self.validated = true;
        }
        retain::<NominalCarrierAuthority>(facts.budget(), 1)?;
        let authority = Arc::new(NominalCarrierAuthority {
            canonical,
            target: Some(target.clone()),
            signature: signature_copy(signature, facts.budget())?,
        });
        let mut view = authority.derive(facts)?;
        view.nominal = Some(authority);
        view.validate_for(facts)?;
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
    fn derive(&self, facts: &ValidatedFacts) -> Result<CarrierContractView> {
        let original = &self.canonical;
        original.validate_for(facts)?;
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
        facts.validate_signature(subject, &self.signature)?;
        let world = facts.world();
        let subject = world.equality_result_subject(subject, facts)?;
        let raw = world.normalized_declaration_schema(original.declaration)?;
        let retained = world.declaration_carriers(original.declaration)?;
        let mut view = world.transport_carrier_parts(
            original.declaration,
            original.arguments.clone(),
            signature_copy(&self.signature, facts.budget())?,
            facts,
            raw,
            retained,
            original.origin.clone(),
            None,
            subject,
        )?;
        // Restricted equality results are recomputed from the transported
        // subject, never copied from the canonical view; `validate_for`
        // rederives and compares them again.
        world.seal_equality_results(&mut view, facts)?;
        Ok(view)
    }

    pub(super) fn validate(
        &self,
        view: &CarrierContractView,
        facts: &ValidatedFacts,
    ) -> Result<()> {
        let expected = self.derive(facts)?;
        if view.declaration != expected.declaration
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
        let snapshot = Arc::new(Snapshot {
            input: Input::Text(if body {
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
                if view.nominal.is_some() {
                    return (function, facts, block, view);
                }
            }
        }
        panic!("CHECK did not publish the expected graph nominal call");
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
