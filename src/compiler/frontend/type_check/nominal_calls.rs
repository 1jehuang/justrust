//! Map canonical call signature values through the same graph reads as the body.
//! Canonical target/signature replay is checked both before and after mapping.
use super::*;

pub(super) struct NominalCallRealization {
    pub(super) target: ir::Callee,
    pub(super) original: ir::Signature,
    pub(super) mapped: ir::Signature,
}

pub(super) enum NominalCallSignature {
    Ready {
        signature: ir::Signature,
        realization: Option<Box<NominalCallRealization>>,
    },
    Blocked(NominalPreparation),
}

impl Checker<'_> {
    pub(super) fn nominal_call_signature(
        &self,
        target: &ir::Callee,
        signature: ir::Signature,
    ) -> Result<NominalCallSignature, String> {
        let Some(inputs) = self.nominal_inputs else {
            return Ok(NominalCallSignature::Ready {
                signature,
                realization: None,
            });
        };
        let plan = self.signatures.plan()?;
        let facts = self
            .facts()
            .validated()
            .ok_or("nominal call signature requires validated caller facts")?;
        facts
            .validate_signature_with(
                ir::SignatureSubject::Call(target),
                &signature,
                &mut self.trait_scratch.borrow_mut(),
            )
            .map_err(|e| e.into_legacy_diagnostic(&plan.budget))?;
        self.infer_storage(signature.params.len())?;
        let mut missing = Vec::new();
        let mut params = Vec::new();
        let mut complete = true;
        for source in &signature.params {
            match call_type(self, inputs, source, &mut missing, 0)? {
                Some(ty) => params.push(ty),
                None => complete = false,
            }
        }
        let result = call_type(self, inputs, &signature.result, &mut missing, 0)?;
        let Some(result) = result.filter(|_| complete) else {
            return Ok(NominalCallSignature::Blocked(NominalPreparation::Blocked {
                nominals: missing.into(),
                declarations: Arc::from([]),
                needs_source_schemas: false,
                unresolved_constraints: false,
            }));
        };
        let mapped = ir::Signature {
            params,
            result,
            // Mapping storage identity does not invent a new carrier contract,
            // source region environment or argument expectation.
            ref_contract: signature.ref_contract.clone(),
        };
        for ty in mapped.params.iter().chain(std::iter::once(&mapped.result)) {
            facts
                .validate_ty(ty)
                .map_err(|e| e.into_legacy_diagnostic(&plan.budget))?;
        }
        facts
            .validate_signature_with(
                ir::SignatureSubject::Call(target),
                &mapped,
                &mut self.trait_scratch.borrow_mut(),
            )
            .map_err(|e| e.into_legacy_diagnostic(&plan.budget))?;
        Ok(NominalCallSignature::Ready {
            signature: mapped.clone(),
            realization: Some(Box::new(NominalCallRealization {
                target: target.clone(),
                original: signature,
                mapped,
            })),
        })
    }
}

fn call_type(
    checker: &Checker<'_>,
    inputs: &NominalPreparationInputs<'_>,
    source: &ir::Ty,
    missing: &mut Vec<NominalDemand>,
    depth: usize,
) -> Result<Option<ir::Ty>, String> {
    checker.infer_work(depth)?;
    if let Some((declaration, arguments)) = nominal_parts(source) {
        if super::nominal_inputs::parent_local::contains_closure(
            arguments,
            &checker.signatures.plan()?.budget,
            depth,
        )? {
            return checker
                .mint_parent_local_nominal(declaration, arguments)
                .map(Some);
        }
        let demand = checker.nominal_input_demand(declaration, arguments)?;
        if let Some(ty) = (inputs.nominal_type)(&demand)? {
            return Ok(Some(ty));
        }
        let mut duplicate = false;
        for prior in missing.iter() {
            checker.infer_work(depth)?;
            if prior.declaration == demand.declaration
                && prior.arguments.len() == demand.arguments.len()
            {
                let mut same = true;
                for (a, b) in prior.arguments.iter().zip(demand.arguments.iter()) {
                    same &= a.same_with(b, depth + 1, &mut |d| checker.infer_work(d))?;
                }
                duplicate |= same;
            }
        }
        if !duplicate {
            checker.infer_storage(1)?;
            missing.push(demand);
        }
        return Ok(None);
    }
    Ok(Some(match source {
        ir::Ty::Ref { kind, referent } => {
            let Some(referent) = call_type(checker, inputs, referent, missing, depth + 1)? else {
                return Ok(None);
            };
            ir::Ty::Ref {
                kind: *kind,
                referent: Arc::new(referent),
            }
        }
        ir::Ty::RawPtr { kind, pointee } => {
            let Some(pointee) = call_type(checker, inputs, pointee, missing, depth + 1)? else {
                return Ok(None);
            };
            ir::Ty::RawPtr {
                kind: *kind,
                pointee: Arc::new(pointee),
            }
        }
        ir::Ty::Array { element, len } => {
            let Some(element) = call_type(checker, inputs, element, missing, depth + 1)? else {
                return Ok(None);
            };
            ir::Ty::Array {
                element: Arc::new(element),
                len: *len,
            }
        }
        ir::Ty::Slice { element } => {
            let Some(element) = call_type(checker, inputs, element, missing, depth + 1)? else {
                return Ok(None);
            };
            ir::Ty::Slice {
                element: Arc::new(element),
            }
        }
        ir::Ty::Tuple(fields) => {
            checker.infer_storage(fields.len())?;
            let mut mapped = Vec::new();
            let mut complete = true;
            for ty in fields.iter() {
                match call_type(checker, inputs, ty, missing, depth + 1)? {
                    Some(ty) => mapped.push(ty),
                    None => complete = false,
                }
            }
            if !complete {
                return Ok(None);
            }
            ir::Ty::Tuple(mapped.into())
        }
        // Retain already canonical rigid projections and closure/dyn identities.
        // This hook grants no new normalization or callable capability.
        other => other.clone(),
    }))
}
