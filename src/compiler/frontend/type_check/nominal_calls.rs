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

/// Graph realizations a trait call resolves against, or the complete demand
/// frontier still missing.
pub(super) enum NominalCallInputs {
    Ready(Vec<ir::Ty>),
    Blocked(NominalPreparation),
}

impl Checker<'_> {
    /// Typed graph realizations for every closed source nominal application
    /// in a trait call's signature whose storage involves a projection, keyed
    /// per declaration and ordered actuals. Reuses the associated-signature
    /// collector: open or unresolved coordinates are never demanded, every
    /// missing read is demanded together before blocking, and trait-call
    /// instantiation selects these through its existing transport without
    /// reminting a nominal's executable fields itself.
    pub(super) fn nominal_trait_call_inputs(
        &self,
        target_trait_call: Option<&ir::TraitCall>,
        signature: &ir::Signature,
    ) -> Result<NominalCallInputs, String> {
        if self.nominal_inputs.is_none() {
            return Ok(NominalCallInputs::Ready(Vec::new()));
        }
        let mut scratch = tr::TraitScratch::default();
        let mut sources = Vec::new();
        let mut missing = Vec::new();
        for root in signature
            .params
            .iter()
            .chain(std::iter::once(&signature.result))
        {
            self.collect_associated_signature_nominals(
                root,
                &mut sources,
                &mut missing,
                &mut scratch,
                false,
                0,
            )?;
        }
        if !missing.is_empty() {
            return Ok(NominalCallInputs::Blocked(NominalPreparation::Blocked {
                nominals: missing.into(),
                declarations: Arc::from([]),
                needs_source_schemas: false,
                unresolved_constraints: false,
            }));
        }
        // The selected impl substitutes the trait operands into its own
        // header, so an operand that is already a typed graph read (such as
        // `Self = Cow<str>`) reappears there as a source application whose
        // storage may name a projection. Offer exactly those existing reads,
        // keyed by declaration and ordered actuals, for selection. Nothing new
        // is demanded here: source operands still go through the collector.
        let mut operands = Vec::new();
        if let Some(call) = target_trait_call {
            self.collect_graph_operand_nominals(&call.trait_ref.self_ty, &mut operands, 0)?;
            for argument in call.trait_ref.args.iter() {
                self.collect_graph_operand_nominals(argument, &mut operands, 0)?;
            }
        }
        let mut realized: Vec<ir::Ty> = Vec::new();
        for source in sources.iter().chain(operands.iter()) {
            let ty = self.realize_constructor_source(source)?;
            let mut duplicate = false;
            let identity = |ty: &ir::Ty| match ty {
                ir::Ty::Struct(value) => Some(value.id()),
                ir::Ty::Enum(value) => Some(value.id()),
                _ => None,
            };
            for previous in realized.iter() {
                self.infer_work(0)?;
                duplicate |= identity(previous).is_some() && identity(previous) == identity(&ty);
            }
            if !duplicate {
                self.infer_storage(1)?;
                realized.push(ty);
            }
        }
        Ok(NominalCallInputs::Ready(realized))
    }

    /// The exact graph reads a realized call signature already names, for its
    /// immutable receipt. Nothing is demanded or minted: these are the reads
    /// `nominal_call_signature` placed into the mapped signature.
    pub(super) fn signature_graph_rows(
        &self,
        signature: &ir::Signature,
    ) -> Result<Option<(Arc<[ir::Ty]>, Option<crate::engine::EngineAuthority>)>, String> {
        let Some(inputs) = self.nominal_inputs else {
            return Ok(None);
        };
        let mut found = Vec::new();
        for root in signature
            .params
            .iter()
            .chain(std::iter::once(&signature.result))
        {
            self.collect_graph_operand_nominals(root, &mut found, 0)?;
        }
        let identity = |ty: &ir::Ty| match ty {
            ir::Ty::Struct(value) => Some(value.id()),
            ir::Ty::Enum(value) => Some(value.id()),
            _ => None,
        };
        let mut rows: Vec<ir::Ty> = Vec::new();
        for ty in found {
            let mut duplicate = false;
            for previous in rows.iter() {
                self.infer_work(0)?;
                duplicate |= identity(previous).is_some() && identity(previous) == identity(&ty);
            }
            if !duplicate {
                self.infer_storage(1)?;
                rows.push(ty);
            }
        }
        if rows.is_empty() {
            return Ok(None);
        }
        Ok(Some((Arc::from(rows), inputs.engine)))
    }

    /// Existing graph-realized nominal applications inside one trait operand.
    fn collect_graph_operand_nominals(
        &self,
        ty: &ir::Ty,
        out: &mut Vec<ir::Ty>,
        depth: usize,
    ) -> Result<(), String> {
        self.infer_work(depth)?;
        match ty {
            ir::Ty::Struct(value) if !value.is_back_edge() => {
                if value.id().index.is_graph() {
                    self.infer_storage(1)?;
                    out.push(ty.clone());
                }
            }
            ir::Ty::Enum(value) if !value.is_back_edge() => {
                if value.id().index.is_graph() {
                    self.infer_storage(1)?;
                    out.push(ty.clone());
                }
            }
            _ => {}
        }
        if let Some((_, arguments)) = nominal_parts(ty) {
            for argument in arguments {
                self.collect_graph_operand_nominals(argument, out, depth + 1)?;
            }
            return Ok(());
        }
        match ty {
            ir::Ty::Tuple(parts) => {
                for part in parts.iter() {
                    self.collect_graph_operand_nominals(part, out, depth + 1)?;
                }
            }
            ir::Ty::Ref {
                referent: child, ..
            }
            | ir::Ty::RawPtr { pointee: child, .. }
            | ir::Ty::Array { element: child, .. }
            | ir::Ty::Slice { element: child } => {
                self.collect_graph_operand_nominals(child, out, depth + 1)?;
            }
            _ => {}
        }
        Ok(())
    }

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
            .validate_source_flow_signature_with_scratch(
                ir::SignatureSubject::Call(target),
                &signature,
                &mut self.trait_scratch.borrow_mut(),
            )
            .map_err(|e| e.into_legacy_diagnostic(&plan.budget))?;
        // Selection may already have realized storage through typed graph
        // reads. Retain the canonical source application independently, not
        // that executable signature, for the later original-source replay.
        // This is exactly the computation replay repeats for this occurrence.
        // Its selected callee is not retained here: the existing receipt
        // replay independently validates target correspondence. A signature
        // without graph reads is already its own original source view.
        let original = if self.signature_graph_rows(&signature)?.is_some() {
            let (_, mut original) = facts
                .canonical_source_call_with_scratch(
                    target,
                    &signature,
                    &mut self.trait_scratch.borrow_mut(),
                )
                .map_err(|e| e.into_legacy_diagnostic(&plan.budget))?;
            // The receipt requires original and mapped to share one contract
            // allocation. When the rebuilt source view's contract differs (for
            // example renamed authored lifetimes), keep the selected signature
            // as the original, exactly as before canonical source views.
            if original.params.len() != signature.params.len()
                || *original.ref_contract != *signature.ref_contract
            {
                signature.clone()
            } else {
                original.ref_contract = signature.ref_contract.clone();
                original
            }
        } else {
            signature.clone()
        };
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
            .validate_source_flow_signature_with_scratch(
                ir::SignatureSubject::Call(target),
                &mapped,
                &mut self.trait_scratch.borrow_mut(),
            )
            .map_err(|e| e.into_legacy_diagnostic(&plan.budget))?;
        Ok(NominalCallSignature::Ready {
            signature: mapped.clone(),
            realization: Some(Box::new(NominalCallRealization {
                target: target.clone(),
                original,
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
