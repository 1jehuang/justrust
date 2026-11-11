//! Occurrence-bound transport of genuine dynamic dispatch through an opaque body.
//! The vtable producer is unchanged. No hidden representation may alter its ABI.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct VirtualPhysicalCall {
    pub(super) publication: source_replay::OriginalBodyId,
    pub(super) occurrence: ir::ExprId,
    source: ir::VirtualCall,
    pub(super) target: ir::VirtualCall,
    public_signature: ir::Signature,
    pub(super) physical_signature: ir::Signature,
}

impl VirtualPhysicalCall {
    pub(super) fn seal(
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
            .ok_or_else(|| Error::new("virtual physical occurrence is outside original body"))?;
        let ir::ExprKind::Call {
            target: ir::Callee::Virtual(source),
            signature,
            ..
        } = &expression.kind
        else {
            return Err(Error::new(
                "physical virtual ABI requires original virtual-call occurrence",
            ));
        };
        let (ir::Callee::Virtual(target), specialized) = specialization.call(occurrence)? else {
            return Err(Error::new(
                "physical virtual call changed its original callee kind",
            ));
        };
        if source.principal().map_err(Error::new)? != target.principal().map_err(Error::new)?
            || source.call.item != target.call.item
            || source.call.trait_ref.declaration != target.call.trait_ref.declaration
        {
            return Err(Error::new(
                "physical virtual call changed canonical principal or item",
            ));
        }
        let world = context.trait_world().map_err(Error::new)?;
        // Keeping a Virtual call requires the exact existing vtable ABI. Opaque
        // normalization cannot manufacture a new virtual slot or hidden ABI.
        for ty in std::iter::once(&target.call.trait_ref.self_ty)
            .chain(target.call.trait_ref.args.iter())
            .chain(target.call.method_args.iter())
            .chain(specialized.params.iter())
            .chain(std::iter::once(&specialized.result))
        {
            budget.charge_work(1).map_err(Error::new)?;
            let physical = representations.normalize(ty, budget)?;
            if !world
                .types_equal(std::slice::from_ref(ty), std::slice::from_ref(&physical))
                .map_err(|error| source_fact_error(error, budget))?
            {
                return Err(Error::new(
                    "unsupported: virtual physicalization changes vtable ABI",
                ));
            }
        }
        let physical_signature = specialized.clone();
        carriers::require_preserved_boundary(
            &specialized,
            &physical_signature,
            &world,
            representations,
            budget,
        )
        .map_err(Error::new)?;
        budget
            .retain_bytes(std::mem::size_of::<Self>())
            .map_err(Error::new)?;
        Ok(Self {
            publication: publication.identity().clone(),
            occurrence,
            source: source.clone(),
            target,
            public_signature: signature.clone(),
            physical_signature,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn physical_virtual_calls_replay_original_occurrence_and_complete_mapping() {
        struct Probe(CompilerProvider);
        impl Provider for Probe {
            fn outputs(&self, key: &Key) -> usize {
                self.0.outputs(key)
            }
            fn start(&self, cx: Context, requested: Key) -> TaskFuture {
                if requested.kind != 9017 {
                    return self.0.start(cx, requested);
                }
                let provider = self.0.clone();
                Box::pin(async move {
                    let facts = Facts { cx: &cx };
                    let types = facts.types().await?;
                    let main = (0..types.callable_count()).find(|i| {
                        types.callable_decl(*i).ok().and_then(|id| types.persistent_decl_path(id).ok())
                            .is_some_and(|p| matches!(p.item(), frontend::PersistentDeclItem::Name(n) if n == "probe"))
                    }).ok_or_else(|| Error::new("missing original main"))?;
                    let template: Arc<TemplateBodyFact> =
                        facts.get(keys::item(TEMPLATE_BODY, main)).await?;
                    let publication = template.publication.as_ref().unwrap();
                    let caller = ir::CallTarget {
                        declaration: types.callable_decl(main).map_err(Error::new)?,
                        type_args: Arc::from([ir::Ty::Int(ir::IntTy::U8)]),
                    };
                    let budget = &provider.0.budget;
                    let lowered = cfg::lower_source_template(publication, budget)
                        .map_err(|e| Error::new(e.to_string()))?;
                    let roots = lowered
                        .function()
                        .locals
                        .iter()
                        .map(|l| l.ty.clone())
                        .collect::<Vec<_>>();
                    let representations = opaque_instances::read(&facts, &roots, budget).await?;
                    let mut physical = normalize(
                        publication,
                        &caller,
                        lowered.function(),
                        &representations,
                        &[],
                        &types,
                        budget,
                    )?;
                    assert!(!physical.proof.virtual_calls.is_empty());
                    let closed = types.instance_facts(&caller).map_err(Error::new)?;
                    physical
                        .seal(
                            publication,
                            lowered.function(),
                            &representations,
                            &types,
                            &closed,
                            budget,
                        )
                        .map_err(Error::new)?;
                    let validation = physical
                        .validation(
                            publication,
                            lowered.function(),
                            &representations,
                            &types,
                            budget,
                        )
                        .map_err(Error::new)?;
                    validation
                        .validate_cfg(physical.function(), &closed)
                        .map_err(Error::new)?;
                    let proof = physical.proof.virtual_calls[0].clone();
                    let callee = ir::Callee::Virtual(proof.target.clone());
                    validation
                        .validate_call(Some(proof.occurrence), &callee, &proof.physical_signature)
                        .map_err(Error::new)?;
                    assert!(
                        validation
                            .validate_call(
                                Some(ir::ExprId(usize::MAX)),
                                &callee,
                                &proof.physical_signature
                            )
                            .is_err()
                    );
                    let mut changed = proof.target.clone();
                    changed.call.item = caller.declaration;
                    assert!(
                        validation
                            .validate_call(
                                Some(proof.occurrence),
                                &ir::Callee::Virtual(changed),
                                &proof.physical_signature
                            )
                            .is_err()
                    );
                    let mut changed = proof.target.clone();
                    changed.call.trait_ref.declaration = caller.declaration;
                    assert!(
                        validation
                            .validate_call(
                                Some(proof.occurrence),
                                &ir::Callee::Virtual(changed),
                                &proof.physical_signature
                            )
                            .is_err()
                    );
                    let mut changed = proof.target.clone();
                    changed.call.trait_ref.args = Arc::from([ir::Ty::Bool]);
                    assert!(
                        validation
                            .validate_call(
                                Some(proof.occurrence),
                                &ir::Callee::Virtual(changed),
                                &proof.physical_signature
                            )
                            .is_err()
                    );
                    let mut changed = proof.target.clone();
                    changed.call.trait_ref.self_ty = ir::Ty::Bool;
                    assert!(
                        validation
                            .validate_call(
                                Some(proof.occurrence),
                                &ir::Callee::Virtual(changed),
                                &proof.physical_signature
                            )
                            .is_err()
                    );
                    let mut changed = proof.target.clone();
                    changed.call.method_args = Arc::from([ir::Ty::Bool]);
                    assert!(
                        validation
                            .validate_call(
                                Some(proof.occurrence),
                                &ir::Callee::Virtual(changed),
                                &proof.physical_signature
                            )
                            .is_err()
                    );
                    let mut signature = proof.physical_signature.clone();
                    signature.result = ir::Ty::Bool;
                    assert!(
                        validation
                            .validate_call(Some(proof.occurrence), &callee, &signature)
                            .is_err()
                    );
                    let mut signature = proof.physical_signature.clone();
                    signature.ref_contract = Arc::new(ir::RefContract::default());
                    assert!(
                        validation
                            .validate_call(Some(proof.occurrence), &callee, &signature)
                            .is_err()
                    );
                    let frontend::PublishedCallableBody::Source(republished) = types
                        .publish_callable_body(caller.declaration, budget)
                        .map_err(|e| source_fact_error(e, budget))?
                    else {
                        panic!("original source")
                    };
                    assert_ne!(publication.identity(), republished.identity());
                    assert!(
                        physical
                            .validation(
                                &republished,
                                lowered.function(),
                                &representations,
                                &types,
                                budget
                            )
                            .is_err()
                    );
                    cx.publish_fact(0, ())
                })
            }
        }
        for workers in [1, 8] {
            let snapshot = Arc::new(Snapshot {
                input: Input::Text("fn probe<T>(hd:&dyn Fn(&str)->String){println!(\"{}\",hd(\"click\"));}fn main(){}".into()),
                source_options: source::SourceOptions::default(), budget: ir::PlanBudget::default(),
            });
            let engine = Engine::new(Probe(CompilerProvider(snapshot)));
            let probe = engine.request(keys::test_key(9017, vec![]));
            engine.run(workers).unwrap();
            probe.completion().unwrap();
        }
    }
}
