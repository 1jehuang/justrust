//! Immutable original/realized call correspondence for graph-backed source typing.
use super::*;

pub(in crate::compiler) mod closed;

fn graph_type(ty: &ir::Ty, budget: &ir::PlanBudget, depth: usize) -> Result<bool, FactsError> {
    sr::work(budget, 1)?;
    budget
        .check_depth(depth)
        .map_err(|e| source_error(budget, &e))?;
    let child = |ty: &ir::Ty| graph_type(ty, budget, depth + 1);
    match ty {
        ir::Ty::Struct(n) => {
            if matches!(n.id().index, ir::NominalIndex::GraphRealization(_)) {
                return Ok(true);
            }
            if n.is_back_edge() {
                return Ok(false);
            }
            for ty in n.def().type_args.iter() {
                if child(ty)? {
                    return Ok(true);
                }
            }
            for field in n.def().fields.iter() {
                if child(&field.ty)? {
                    return Ok(true);
                }
            }
        }
        ir::Ty::Enum(n) => {
            if matches!(n.id().index, ir::NominalIndex::GraphRealization(_)) {
                return Ok(true);
            }
            if n.is_back_edge() {
                return Ok(false);
            }
            for ty in n.def().type_args.iter() {
                if child(ty)? {
                    return Ok(true);
                }
            }
            for variant in n.def().variants.iter() {
                for field in variant.fields.iter() {
                    if child(&field.ty)? {
                        return Ok(true);
                    }
                }
            }
        }
        ir::Ty::Tuple(types) => {
            for ty in types.iter() {
                if child(ty)? {
                    return Ok(true);
                }
            }
        }
        ir::Ty::Ref { referent: ty, .. }
        | ir::Ty::RawPtr { pointee: ty, .. }
        | ir::Ty::Array { element: ty, .. }
        | ir::Ty::Slice { element: ty } => return child(ty),
        ir::Ty::Closure(c) => {
            for ty in c.types() {
                if child(ty)? {
                    return Ok(true);
                }
            }
        }
        ir::Ty::Dyn(d) => {
            for ty in d.types() {
                if child(ty)? {
                    return Ok(true);
                }
            }
        }
        ir::Ty::AssociatedType(p) => {
            if child(&p.trait_ref.self_ty)? {
                return Ok(true);
            }
            for ty in p.trait_ref.args.iter() {
                if child(ty)? {
                    return Ok(true);
                }
            }
        }
        _ => {}
    }
    Ok(false)
}

#[derive(Clone, Debug)]
pub(super) struct Receipt {
    expression: ir::ExprId,
    target: ir::Callee,
    original: ir::Signature,
    mapped: ir::Signature,
}

fn validate(
    receipt: &Receipt,
    facts: &tr::ValidatedFacts,
    budget: &ir::PlanBudget,
    scratch: &mut tr::TraitScratch,
) -> Result<(), FactsError> {
    facts.validate_signature_with(
        ir::SignatureSubject::Call(&receipt.target),
        &receipt.original,
        scratch,
    )?;
    facts.validate_signature_with(
        ir::SignatureSubject::Call(&receipt.target),
        &receipt.mapped,
        scratch,
    )?;
    if !Arc::ptr_eq(&receipt.original.ref_contract, &receipt.mapped.ref_contract)
        || receipt.original.params.len() != receipt.mapped.params.len()
    {
        return Err(FactsError::invalid(
            budget,
            "nominal call realization changed contract or arity",
        ));
    }
    for (source, mapped) in receipt
        .original
        .params
        .iter()
        .zip(&receipt.mapped.params)
        .chain(std::iter::once((
            &receipt.original.result,
            &receipt.mapped.result,
        )))
    {
        facts.validate_ty_metadata_scratch(source, budget, scratch)?;
        facts.validate_ty_metadata_scratch(mapped, budget, scratch)?;
        if !facts
            .world()
            .types_equal(std::slice::from_ref(source), std::slice::from_ref(mapped))?
        {
            return Err(FactsError::invalid(
                budget,
                "nominal call realization changed semantic type",
            ));
        }
    }
    Ok(())
}

impl CanonicalSourceCapability {
    pub(super) fn record_nominal_call_realization(
        &self,
        expression: ir::ExprId,
        target: &ir::Callee,
        original: ir::Signature,
        mapped: ir::Signature,
        scratch: &mut tr::TraitScratch,
    ) -> Result<(), String> {
        let budget = &self.plan.budget;
        let receipt = Receipt {
            expression,
            target: target.clone(),
            original,
            mapped,
        };
        validate(&receipt, &self.facts, budget, scratch).map_err(|e| self.render(e))?;
        let mut inventory = self.nominal_call_realizations.borrow_mut();
        let rows = inventory
            .as_mut()
            .ok_or("nominal call realization outside graph-backed source consumer")?;
        for row in rows.iter() {
            budget.charge_work(1)?;
            if row.expression == expression {
                return Err("duplicate nominal call realization".into());
            }
        }
        budget.retain_bytes(std::mem::size_of::<Receipt>())?;
        rows.try_reserve(1)
            .map_err(|_| "nominal call receipt allocation failed")?;
        rows.push(receipt);
        Ok(())
    }

    pub(in crate::compiler) fn realized_source_call_signature(
        &self,
        expression: ir::ExprId,
        target: &ir::Callee,
        canonical: &ir::Signature,
        current: &ir::Signature,
        scratch: &mut tr::TraitScratch,
    ) -> Result<Option<ir::Signature>, FactsError> {
        let budget = &self.plan.budget;
        let inventory = self.nominal_call_realizations.borrow();
        let Some(rows) = inventory.as_ref() else {
            return Ok(None);
        };
        let row = rows
            .iter()
            .find(|row| row.expression == expression)
            .ok_or_else(|| {
                FactsError::invalid(
                    budget,
                    "nominal call realization omitted before canonical rebuild",
                )
            })?;
        validate(row, &self.facts, budget, scratch)?;
        validate_original_callee(&row.target, target, budget)?;
        self.facts.validate_signature_with(
            ir::SignatureSubject::Call(target),
            canonical,
            scratch,
        )?;
        let mut same_original = row.original.params.len() == canonical.params.len()
            && row.original.ref_contract == canonical.ref_contract;
        for (retained, fresh) in row
            .original
            .params
            .iter()
            .zip(&canonical.params)
            .chain(std::iter::once((&row.original.result, &canonical.result)))
        {
            same_original &= replay_type_equal(retained, fresh, budget)?;
        }
        // A rebuild (e.g. a second call of the same closure) allocates equal
        // types afresh, so the retained realization is compared structurally.
        let same_realization =
            structurally_same_call(&row.target, &row.mapped, target, current, budget)?;
        if !same_original || !same_realization {
            return Err(FactsError::invalid(
                budget,
                "nominal call realization changed target or signature",
            ));
        }
        Ok(Some(row.mapped.clone()))
    }
}

/// Structural (not pointer) equality of a callee and its realized signature.
/// Canonical rebuilds allocate equal type DAGs afresh; identity of allocation
/// is not part of the realization's meaning.
fn structurally_same_call(
    a: &ir::Callee,
    sa: &ir::Signature,
    b: &ir::Callee,
    sb: &ir::Signature,
    budget: &ir::PlanBudget,
) -> Result<bool, FactsError> {
    if std::mem::discriminant(a) != std::mem::discriminant(b)
        || a.metadata_types().count() != b.metadata_types().count()
        || sa.params.len() != sb.params.len()
        || sa.ref_contract != sb.ref_contract
    {
        return Ok(false);
    }
    for (left, right) in a
        .metadata_types()
        .zip(b.metadata_types())
        .chain(sa.params.iter().zip(&sb.params))
        .chain(std::iter::once((&sa.result, &sb.result)))
    {
        if !replay_type_equal(left, right, budget)? {
            return Ok(false);
        }
    }
    Ok(true)
}

pub(super) fn replay(
    source: &OriginalSource,
    checked: &ir::CheckedFunction,
    facts: &tr::ValidatedFacts,
    budget: &ir::PlanBudget,
) -> Result<(), FactsError> {
    let Some(rows) = &source.nominal_call_realizations else {
        // Legacy publications legitimately have no realization inventory. A
        // graph-backed call cannot opt into that path by deleting its receipts.
        // Derive the requirement from the actual canonical call payload and
        // ordered target actuals, independently of the optional receipt data.
        for expression in &checked.exprs {
            sr::work(budget, 1)?;
            if let ir::ExprKind::Call {
                target, signature, ..
            } = &expression.kind
            {
                facts.validate_signature(ir::SignatureSubject::Call(target), signature)?;
                for ty in target
                    .metadata_types()
                    .chain(signature.params.iter())
                    .chain(std::iter::once(&signature.result))
                {
                    if graph_type(ty, budget, 0)? {
                        return Err(FactsError::invalid(
                            budget,
                            "graph-backed source call omitted realization inventory",
                        ));
                    }
                }
            }
        }
        return Ok(());
    };
    if rows.len() != source.call_mapping.len() {
        return Err(FactsError::invalid(
            budget,
            "nominal call realization inventory incomplete",
        ));
    }
    // One proof/replay scratch for this replay computation only.
    let mut scratch = tr::TraitScratch::default();
    for (row, (_, expression, target)) in rows.iter().zip(&source.call_mapping) {
        sr::work(budget, 1)?;
        if row.expression != *expression {
            return Err(FactsError::invalid(
                budget,
                "nominal call realization occurrence order changed",
            ));
        }
        let call = checked
            .exprs
            .get(expression.0)
            .ok_or_else(|| FactsError::invalid(budget, "nominal call realization outside arena"))?;
        let ir::ExprKind::Call {
            target: actual_target,
            signature,
            ..
        } = &call.kind
        else {
            return Err(FactsError::invalid(
                budget,
                "nominal call realization mapped to non-call",
            ));
        };
        validate_original_callee(target, &row.target, budget)?;
        validate(row, facts, budget, &mut scratch)?;
        if !structurally_same_call(&row.target, &row.mapped, actual_target, signature, budget)? {
            return Err(FactsError::invalid(
                budget,
                "nominal call realization payload changed",
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph_publication() -> (ir::PlanBudget, Arc<sr::CanonicalSourcePublication>) {
        use crate::compiler::{CompilerProvider, Input, Snapshot, TEMPLATE_BODY, TemplateBodyFact};
        use crate::engine::{Engine, Key, Provider};
        struct PublicationProvider(Arc<Snapshot>);
        impl Provider for PublicationProvider {
            fn outputs(&self, key: &Key) -> usize {
                if key.kind == 998 {
                    1
                } else {
                    CompilerProvider(self.0.clone()).outputs(key)
                }
            }
            fn start(&self, cx: crate::engine::Context, key: Key) -> crate::engine::TaskFuture {
                if key.kind != 998 {
                    return CompilerProvider(self.0.clone()).start(cx, key);
                }
                Box::pin(async move {
                    let signatures = cx
                        .request(crate::compiler::keys::SignaturesDebtRoot.key())
                        .get(0);
                    let resolved = cx
                        .request(crate::compiler::keys::ResolveDebtRoot.key())
                        .get(0);
                    let plan: Arc<PlanCtx> = cx.read_fact(signatures).await?;
                    let program: Arc<Arc<Program>> = cx.read_fact(resolved).await?;
                    // The unchanged accepted Peek program demands native_heap::release.
                    // Its actual release -> deallocate call exercises the graph/source
                    // signature correspondence that the main body's enum constructors do not.
                    let candidates = program
                        .functions
                        .iter()
                        .enumerate()
                        .filter(|(_, f)| {
                            f.name.segments().last().is_some_and(|name| {
                                program.names.spelling(*name).unwrap() == "release"
                            })
                        })
                        .map(|(i, _)| i)
                        .collect::<Vec<_>>();
                    assert_eq!(
                        candidates.len(),
                        1,
                        "exact retained release source declaration"
                    );
                    let index = candidates[0];
                    let template = cx
                        .request(crate::compiler::keys::Item::<{ TEMPLATE_BODY }>(index).key())
                        .get(0);
                    let body: Arc<TemplateBodyFact> = cx.read_fact(template).await?;
                    let publication = body
                        .publication
                        .as_ref()
                        .expect("real canonical Source publication")
                        .clone();
                    assert_eq!(
                        publication.original().source_for_test().declaration,
                        plan.function_decl(index).unwrap()
                    );
                    cx.publish_fact(0, publication)
                })
            }
        }
        let budget = ir::PlanBudget::default();
        let snapshot = Arc::new(Snapshot {
            input: Input::Text("trait It{type Item;}struct C;impl It for C{type Item=i32;}struct Peek<I:It>{item:Option<Option<I::Item>>}fn main(){let p:Peek<C>=Peek{item:Some(Some(7))};if let Some(Some(n))=p.item{println!(\"{}\",n);}}".into()),
            source_options: crate::compiler::source::SourceOptions::default(),
            budget: budget.clone(),
        });
        let engine = Engine::new(PublicationProvider(snapshot));
        let demand = engine.request(crate::compiler::keys::test_key(998, vec![]));
        engine.run(1).unwrap();
        demand.completion().unwrap();
        let publication = demand
            .fact::<Arc<sr::CanonicalSourcePublication>>(0)
            .unwrap()
            .as_ref()
            .clone();
        (budget, publication)
    }

    #[test]
    fn graph_source_publication_rejects_missing_and_changed_call_receipts() {
        let (budget, publication) = graph_publication();
        let source = publication.source();
        publication
            .validate_source(source, publication.facts(), &budget)
            .unwrap();
        let original = publication.original().source_for_test();
        assert!(
            !original
                .nominal_call_realizations
                .as_ref()
                .expect("graph source receipt mode")
                .is_empty()
        );
        for mode in 0..4 {
            let mut changed = original.clone();
            match mode {
                0 => changed.nominal_call_realizations = None,
                1 => {
                    changed.nominal_call_realizations.as_mut().unwrap().pop();
                }
                2 => {
                    let rows = changed.nominal_call_realizations.as_mut().unwrap();
                    rows.push(rows[0].clone());
                }
                _ => {
                    let rows = changed.nominal_call_realizations.as_mut().unwrap();
                    rows[0].expression = ir::ExprId(source.exprs.len());
                }
            }
            assert!(
                replay_source_body(&changed, source, publication.facts(), &budget).is_err(),
                "real source publication accepted receipt forgery mode {mode}"
            );
        }
    }
}
