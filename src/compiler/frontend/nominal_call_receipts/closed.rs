//! Closed instantiation reuses the source-call receipt payload and validators.
use super::*;

#[derive(Clone, Debug)]
pub(in crate::compiler) struct ClosedReceipt {
    receipt: Receipt,
    selection: ir::Callee,
    engine: crate::engine::EngineAuthority,
    conversions: Arc<[ir::NominalCarrierMapping]>,
}

pub(in crate::compiler) fn conversion_rows(
    original: &ir::Signature,
    mapped: &ir::Signature,
    budget: &ir::PlanBudget,
) -> Result<Vec<ir::NominalCarrierMapping>, String> {
    fn walk(
        a: &ir::Ty,
        b: &ir::Ty,
        budget: &ir::PlanBudget,
        depth: usize,
        rows: &mut Vec<ir::NominalCarrierMapping>,
    ) -> Result<(), String> {
        budget.check_depth(depth)?;
        budget.charge_work(1)?;
        let pair = match (a, b) {
            (ir::Ty::Struct(a), ir::Ty::Struct(b)) => Some((
                a.id(),
                b.id(),
                a.def().type_args.as_ref(),
                b.def().type_args.as_ref(),
            )),
            (ir::Ty::Enum(a), ir::Ty::Enum(b)) => Some((
                a.id(),
                b.id(),
                a.def().type_args.as_ref(),
                b.def().type_args.as_ref(),
            )),
            _ => None,
        };
        if let Some((original, instantiated, a, b)) = pair {
            if a.len() != b.len() {
                return Err("closed signature conversion arity changed".into());
            }
            if original != instantiated
                || matches!(original.index, ir::NominalIndex::GraphRealization(_))
            {
                budget.retain_bytes(std::mem::size_of::<ir::NominalCarrierMapping>())?;
                rows.try_reserve(1)
                    .map_err(|_| "closed conversion inventory allocation failed")?;
                rows.push(ir::NominalCarrierMapping {
                    original,
                    instantiated,
                });
            }
            for (a, b) in a.iter().zip(b) {
                walk(a, b, budget, depth + 1, rows)?;
            }
            return Ok(());
        }
        match (a, b) {
            (ir::Ty::Ref { referent: a, .. }, ir::Ty::Ref { referent: b, .. })
            | (ir::Ty::RawPtr { pointee: a, .. }, ir::Ty::RawPtr { pointee: b, .. })
            | (ir::Ty::Array { element: a, .. }, ir::Ty::Array { element: b, .. })
            | (ir::Ty::Slice { element: a }, ir::Ty::Slice { element: b }) => {
                walk(a, b, budget, depth + 1, rows)
            }
            (ir::Ty::Tuple(a), ir::Ty::Tuple(b)) if a.len() == b.len() => {
                for (a, b) in a.iter().zip(b.iter()) {
                    walk(a, b, budget, depth + 1, rows)?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
    if original.params.len() != mapped.params.len() {
        return Err("closed signature conversion parameter count changed".into());
    }
    let mut rows = Vec::new();
    for (a, b) in original
        .params
        .iter()
        .zip(&mapped.params)
        .chain(std::iter::once((&original.result, &mapped.result)))
    {
        walk(a, b, budget, 0, &mut rows)?;
    }
    Ok(rows)
}

impl ClosedReceipt {
    pub(in crate::compiler) fn validate_current_engine(
        &self,
        engine: crate::engine::EngineAuthority,
    ) -> Result<(), String> {
        if self.engine != engine {
            return Err("foreign engine closed call receipt".into());
        }
        Ok(())
    }
    pub(in crate::compiler) fn issue(
        expression: ir::ExprId,
        selection: &ir::Callee,
        target: &ir::Callee,
        original: &ir::Signature,
        mapped: &ir::Signature,
        engine: crate::engine::EngineAuthority,
        facts: &tr::ValidatedFacts,
        budget: &ir::PlanBudget,
    ) -> Result<Self, String> {
        let receipt = Receipt {
            expression,
            target: target.clone(),
            original: original.clone(),
            mapped: mapped.clone(),
        };
        validate(&receipt, facts, budget).map_err(|e| e.into_legacy_diagnostic(budget))?;
        let output = Self {
            receipt,
            selection: selection.clone(),
            engine,
            conversions: conversion_rows(original, mapped, budget)?.into(),
        };
        output.validate(facts, budget)?;
        Ok(output)
    }
    fn validate(&self, facts: &tr::ValidatedFacts, budget: &ir::PlanBudget) -> Result<(), String> {
        validate(&self.receipt, facts, budget).map_err(|e| e.into_legacy_diagnostic(budget))?;
        let selected = match &self.selection {
            ir::Callee::Direct(target) => ir::Callee::Direct(
                facts
                    .service()
                    .discharge_direct(target, facts)
                    .map_err(|e| e.into_legacy_diagnostic(budget))?
                    .target()
                    .clone(),
            ),
            ir::Callee::TraitItem(call) => facts
                .service()
                .resolve_trait_call(call, facts)
                .map_err(|e| e.into_legacy_diagnostic(budget))?
                .callee()
                .clone(),
            ir::Callee::Virtual(call) => ir::Callee::Virtual(call.clone()),
        };
        validate_original_callee(&selected, &self.receipt.target, budget)
            .map_err(|e| e.into_legacy_diagnostic(budget))?;
        let derived = conversion_rows(&self.receipt.original, &self.receipt.mapped, budget)?;
        if derived.len() != self.conversions.len()
            || derived
                .iter()
                .zip(self.conversions.iter())
                .any(|(a, b)| a.original != b.original || a.instantiated != b.instantiated)
        {
            return Err("closed signature conversion inventory changed".into());
        }
        super::super::super::nominal_realization::substitution::replay_closed_signature(
            self.engine,
            &self.receipt.target,
            &self.receipt.original,
            &self.receipt.mapped,
            ir::FactsCtx::from_validated(facts),
            budget,
        )?;
        Ok(())
    }
}

pub(in crate::compiler) fn replay(
    rows: Option<&[ClosedReceipt]>,
    engine: Option<crate::engine::EngineAuthority>,
    checked: &ir::CheckedFunction,
    template: &crate::compiler::ownership::VerifiedTemplateCfg,
    arguments: &[ir::Ty],
    facts: &tr::ValidatedFacts,
    budget: &ir::PlanBudget,
) -> Result<(), String> {
    let calls = checked
        .exprs
        .iter()
        .enumerate()
        .filter_map(|(index, e)| match &e.kind {
            ir::ExprKind::Call {
                target, signature, ..
            } => Some((ir::ExprId(index), target, signature)),
            _ => None,
        });
    let mut originals =
        template
            .source()
            .exprs
            .iter()
            .enumerate()
            .filter_map(|(index, expression)| match &expression.kind {
                ir::ExprKind::Call { target, .. } => Some((ir::ExprId(index), target)),
                _ => None,
            });
    let Some(rows) = rows else {
        if engine.is_some() && originals.clone().next().is_some() {
            return Err("closed call omitted required recorded inventory".into());
        }
        for (_, target, signature) in calls {
            for ty in target
                .metadata_types()
                .chain(signature.params.iter())
                .chain(std::iter::once(&signature.result))
            {
                if graph_type(ty, budget, 0).map_err(|e| e.into_legacy_diagnostic(budget))? {
                    return Err("closed graph call omitted realization inventory".into());
                }
            }
        }
        return Ok(());
    };
    let mut count = 0;
    for (expression, target, signature) in calls {
        budget.charge_work(1)?;
        let row = rows
            .get(count)
            .ok_or("closed call realization inventory incomplete")?;
        if row.receipt.expression != expression {
            return Err("closed call realization occurrence order changed".into());
        }
        let (original_expression, original_selection) = originals
            .next()
            .ok_or("closed call absent from original template inventory")?;
        if original_expression != expression {
            return Err("closed call changed original template occurrence".into());
        }
        facts
            .world()
            .replay_nominal_call_selection(
                original_selection,
                &row.selection,
                template.env(),
                arguments,
            )
            .map_err(|e| e.into_legacy_diagnostic(budget))?;
        row.validate_current_engine(engine.ok_or("closed call receipt omitted engine authority")?)?;
        row.validate(facts, budget)?;
        if !ir::identical_call_types(
            &row.receipt.target,
            &row.receipt.mapped,
            target,
            signature,
            budget,
        )? {
            return Err("closed call realization payload changed".into());
        }
        count += 1;
    }
    if count != rows.len() || originals.next().is_some() {
        return Err("closed call realization inventory has extra rows".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::ir::closed_call_receipt_tests::{
        issued_body_fixture, issued_rows, reject_changed_rows,
    };

    #[test]
    fn issued_closed_call_body_rejects_changed_receipt_inventory() {
        let source = "fn raw<U>()->u32{1}fn forward<T>(v:T)->T{let a=raw::<u32>();let b=raw::<u64>();v}fn main(){let n=forward(7u32);}";
        let (body, engine) = issued_body_fixture(source);
        let rows = issued_rows(&body);
        assert_eq!(rows.len(), 2, "two original call occurrences required");
        assert!(!reject_changed_rows(&body, engine, Some(rows.clone())));
        let (_, foreign_engine) = issued_body_fixture(source);
        assert_ne!(engine, foreign_engine);

        // Each mutation is consumed through the actual typed BODY boundary and
        // its original-template substitution proof, not just a row validator.
        assert!(reject_changed_rows(&body, engine, None));
        let mut cases: Vec<(&str, Vec<ClosedReceipt>)> = Vec::new();
        cases.push(("empty", Vec::new()));
        cases.push(("missing", vec![rows[0].clone()]));
        let mut extra = rows.to_vec();
        extra.push(rows[0].clone());
        cases.push(("extra", extra));
        let mut reordered = rows.to_vec();
        reordered.swap(0, 1);
        cases.push(("order", reordered));
        let mut wrong_expression = rows.to_vec();
        wrong_expression[0].receipt.expression = rows[1].receipt.expression;
        cases.push(("expression", wrong_expression));
        let mut wrong_callee = rows.to_vec();
        let ir::Callee::Direct(target) = &mut wrong_callee[0].receipt.target else {
            panic!("fixture raw call is direct")
        };
        target.declaration = body.facts.declaration();
        cases.push(("callee", wrong_callee));
        let mut wrong_actual = rows.to_vec();
        let ir::Callee::Direct(target) = &mut wrong_actual[0].receipt.target else {
            panic!("fixture raw call is direct")
        };
        assert_eq!(target.type_args.len(), 1);
        target.type_args = Arc::from([ir::Ty::Int(ir::IntTy::U16)]);
        cases.push(("actual", wrong_actual));
        let mut wrong_selection = rows.to_vec();
        let ir::Callee::Direct(target) = &mut wrong_selection[0].selection else {
            panic!("fixture selection is direct")
        };
        target.type_args = Arc::from([ir::Ty::Int(ir::IntTy::U16)]);
        cases.push(("original selection substitution", wrong_selection));
        let mut wrong_contract = rows.to_vec();
        let mut contract = wrong_contract[0]
            .receipt
            .mapped
            .ref_contract
            .as_ref()
            .clone();
        contract.region_count += 1;
        wrong_contract[0].receipt.mapped.ref_contract = Arc::new(contract);
        cases.push(("contract", wrong_contract));
        let mut foreign = rows.to_vec();
        foreign[0].engine = foreign_engine;
        cases.push(("foreign engine", foreign));
        for (name, rows) in cases {
            assert!(
                reject_changed_rows(&body, engine, Some(rows.into())),
                "actual BODY accepted changed {name} receipt inventory"
            );
        }
    }
}
