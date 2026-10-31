//! Resolve only the generic slots read by an associated projection. An
//! unrelated unresolved slot never supplies fabricated normalization evidence.
use super::*;

pub(super) enum ProjectedConstraintProgress {
    Complete,
    Pending,
    Conflict(FactsError),
}

impl Inference<'_, '_> {
    pub(super) fn projected_schema(
        &self,
        ty: &ir::Ty,
        depth: usize,
        defaults: bool,
    ) -> Result<Option<InferTerm>, String> {
        let term = self.schema(ty, depth)?;
        self.normalize_schema_projections(&term, depth + 1, defaults)
    }

    fn normalize_schema_projections(
        &self,
        term: &InferTerm,
        depth: usize,
        defaults: bool,
    ) -> Result<Option<InferTerm>, String> {
        self.checker.infer_work(depth)?;
        let kind = match term.as_ref() {
            InferKind::Projection(declaration, item, children) => {
                self.checker.infer_storage(children.len())?;
                let mut actuals = Vec::new();
                for child in children.iter() {
                    let Some(child) =
                        self.normalize_schema_projections(child, depth + 1, defaults)?
                    else {
                        return Ok(None);
                    };
                    let Some(actual) =
                        self.resolve_projection_input(&child, depth + 1, defaults)?
                    else {
                        // Only independently constrained input slots may close
                        // a projection. Its expected output never supplies them.
                        return Ok(None);
                    };
                    actuals.push(actual);
                }
                let mut actuals = actuals.into_iter();
                let receiver = actuals.next().ok_or("projection inference lost receiver")?;
                let raw = ir::Ty::AssociatedType(Arc::new(ir::AssociatedTypeProjection {
                    trait_ref: ir::TraitRef {
                        declaration: *declaration,
                        self_ty: receiver,
                        args: actuals.collect(),
                    },
                    item: *item,
                }));
                let plan = self.checker.signatures.plan()?;
                let caller = self.checker.facts();
                let facts = caller.validated().ok_or(
                    "unsupported: projected call inference requires validated caller facts",
                )?;
                let normal = facts
                    .normalize_substituted_projection_type(&raw, &plan.budget)
                    .map_err(|error| error.into_legacy_diagnostic(&plan.budget))?;
                return self
                    .checker
                    .infer_type_term(&normal, None, depth + 1)
                    .map(Some);
            }
            InferKind::Rigid(ir::Ty::AssociatedType(projection)) => {
                let Some(receiver) =
                    self.projected_schema(&projection.trait_ref.self_ty, depth + 1, defaults)?
                else {
                    return Ok(None);
                };
                let Some(receiver) =
                    self.resolve_projection_input(&receiver, depth + 1, defaults)?
                else {
                    return Ok(None);
                };
                let mut arguments = Vec::new();
                self.checker
                    .infer_storage(projection.trait_ref.args.len())?;
                for argument in projection.trait_ref.args.iter() {
                    let Some(term) = self.projected_schema(argument, depth + 1, defaults)? else {
                        return Ok(None);
                    };
                    let Some(actual) = self.resolve_projection_input(&term, depth + 1, defaults)?
                    else {
                        return Ok(None);
                    };
                    arguments.push(actual);
                }
                let raw = ir::Ty::AssociatedType(Arc::new(ir::AssociatedTypeProjection {
                    trait_ref: ir::TraitRef {
                        declaration: projection.trait_ref.declaration,
                        self_ty: receiver,
                        args: arguments.into(),
                    },
                    item: projection.item,
                }));
                let plan = self.checker.signatures.plan()?;
                let caller = self.checker.facts();
                let Some(facts) = caller.validated() else {
                    return Err(
                        "unsupported: projected call inference requires validated caller facts"
                            .into(),
                    );
                };
                let normal = facts
                    .normalize_substituted_projection_type(&raw, &plan.budget)
                    .map_err(|error| error.into_legacy_diagnostic(&plan.budget))?;
                return self
                    .checker
                    .infer_type_term(&normal, None, depth + 1)
                    .map(Some);
            }
            InferKind::PhantomData(declaration, argument) => {
                let Some(argument) =
                    self.normalize_schema_projections(argument, depth + 1, defaults)?
                else {
                    return Ok(None);
                };
                InferKind::PhantomData(*declaration, argument)
            }
            InferKind::FnPointer(inputs, output) => {
                let budget = &self.checker.signatures.plan()?.budget;
                let mut mapped = ir::FnPointerTy::storage(inputs.len(), budget)?;
                for t in inputs.iter() {
                    let Some(t) = self.normalize_schema_projections(t, depth + 1, defaults)? else {
                        return Ok(None);
                    };
                    mapped.push(t);
                }
                let Some(output) =
                    self.normalize_schema_projections(output, depth + 1, defaults)?
                else {
                    return Ok(None);
                };
                InferKind::FnPointer(ir::FnPointerTy::slice(mapped, budget)?, output)
            }
            InferKind::Tuple(fields)
            | InferKind::Nominal(_, fields)
            | InferKind::Opaque(_, fields) => {
                self.checker.infer_storage(fields.len())?;
                let mut mapped = Vec::new();
                for field in fields.iter() {
                    let Some(field) =
                        self.normalize_schema_projections(field, depth + 1, defaults)?
                    else {
                        return Ok(None);
                    };
                    mapped.push(field);
                }
                match term.as_ref() {
                    InferKind::Nominal(declaration, _) => {
                        InferKind::Nominal(*declaration, mapped.into())
                    }
                    InferKind::Opaque(descriptor, _) => {
                        InferKind::Opaque(descriptor.clone(), mapped.into())
                    }
                    _ => InferKind::Tuple(mapped.into()),
                }
            }
            InferKind::Ref(kind, child) | InferKind::RawPtr(kind, child) => {
                let Some(child) = self.normalize_schema_projections(child, depth + 1, defaults)?
                else {
                    return Ok(None);
                };
                if matches!(term.as_ref(), InferKind::Ref(..)) {
                    InferKind::Ref(*kind, child)
                } else {
                    InferKind::RawPtr(*kind, child)
                }
            }
            InferKind::Array(child, len) => {
                let Some(child) = self.normalize_schema_projections(child, depth + 1, defaults)?
                else {
                    return Ok(None);
                };
                InferKind::Array(child, *len)
            }
            InferKind::Slice(child) => {
                let Some(child) = self.normalize_schema_projections(child, depth + 1, defaults)?
                else {
                    return Ok(None);
                };
                InferKind::Slice(child)
            }
            _ => return Ok(Some(term.clone())),
        };
        self.checker.infer_node(kind).map(Some)
    }

    fn resolve_projection_input(
        &self,
        term: &InferTerm,
        depth: usize,
        defaults: bool,
    ) -> Result<Option<ir::Ty>, String> {
        self.checker.infer_work(depth)?;
        if let InferKind::Slot(index) = term.as_ref() {
            return match self.core.slots().get(*index).and_then(Option::as_ref) {
                Some(bound) => self.resolve_projection_input(bound, depth + 1, defaults),
                None => Ok(None),
            };
        }
        let map = |child: &InferTerm| self.resolve_projection_input(child, depth + 1, defaults);
        let ty = match term.as_ref() {
            InferKind::Projection(..) => {
                let Some(normal) = self.normalize_schema_projections(term, depth + 1, defaults)?
                else {
                    return Ok(None);
                };
                return self.resolve(&normal, defaults, depth + 1);
            }
            InferKind::PhantomData(declaration, argument) => {
                let Some(argument) = map(argument)? else {
                    return Ok(None);
                };
                return self
                    .materialize(*declaration, &[argument])
                    .map(Some)
                    .map_err(|error| {
                        error.into_legacy_diagnostic(
                            &self.checker.signatures.plan.as_ref().unwrap().budget,
                        )
                    });
            }
            InferKind::FnPointer(inputs, output) => {
                let budget = &self.checker.signatures.plan()?.budget;
                let mut mapped = ir::FnPointerTy::storage(inputs.len(), budget)?;
                for t in inputs.iter() {
                    let Some(t) = map(t)? else {
                        return Ok(None);
                    };
                    mapped.push(t);
                }
                let Some(output) = map(output)? else {
                    return Ok(None);
                };
                ir::Ty::FnPointer(ir::FnPointerTy::new(mapped, output, budget)?)
            }
            InferKind::Tuple(fields)
            | InferKind::Nominal(_, fields)
            | InferKind::Opaque(_, fields) => {
                self.checker.infer_storage(fields.len())?;
                let mut mapped = Vec::new();
                for field in fields.iter() {
                    let Some(field) = map(field)? else {
                        return Ok(None);
                    };
                    mapped.push(field);
                }
                if let InferKind::Nominal(declaration, _) = term.as_ref() {
                    let plan = self.checker.signatures.plan()?;
                    return self
                        .materialize(*declaration, &mapped)
                        .map(Some)
                        .map_err(|error| error.into_legacy_diagnostic(&plan.budget));
                }
                if let InferKind::Opaque(descriptor, _) = term.as_ref() {
                    let mut mapped = mapped.into_iter();
                    let descriptor = descriptor.try_map(|_| {
                        self.checker.infer_work(depth + 1)?;
                        mapped
                            .next()
                            .ok_or_else(|| "opaque projection lost a child".to_string())
                    })?;
                    if mapped.next().is_some() {
                        return Err("opaque projection retained extra children".into());
                    }
                    return Ok(Some(ir::Ty::Opaque(Arc::new(descriptor))));
                }
                ir::Ty::Tuple(mapped.into())
            }
            InferKind::Ref(kind, child) | InferKind::RawPtr(kind, child) => {
                let Some(child) = map(child)? else {
                    return Ok(None);
                };
                if matches!(term.as_ref(), InferKind::Ref(..)) {
                    ir::Ty::Ref {
                        kind: *kind,
                        referent: Arc::new(child),
                    }
                } else {
                    ir::Ty::RawPtr {
                        kind: *kind,
                        pointee: Arc::new(child),
                    }
                }
            }
            InferKind::Array(child, len) => {
                let Some(element) = map(child)? else {
                    return Ok(None);
                };
                ir::Ty::Array {
                    element: Arc::new(element),
                    len: *len,
                }
            }
            InferKind::Slice(child) => {
                let Some(element) = map(child)? else {
                    return Ok(None);
                };
                ir::Ty::Slice {
                    element: Arc::new(element),
                }
            }
            _ => return self.resolve(term, defaults, depth + 1),
        };
        Ok(Some(ty))
    }

    pub(super) fn constrain_projected_formals(
        &mut self,
        constraints: Vec<(&ir::Ty, InferTerm)>,
        fallback: bool,
    ) -> Result<bool, String> {
        let mut terms = Vec::new();
        self.checker.infer_storage(constraints.len())?;
        for (formal, actual) in constraints {
            terms.push((self.schema(formal, 0)?, actual));
        }
        self.constrain_projected_terms(terms, fallback)
    }

    pub(super) fn constrain_projected_terms(
        &mut self,
        constraints: Vec<(InferTerm, InferTerm)>,
        fallback: bool,
    ) -> Result<bool, String> {
        match self.constrain_projected_terms_progress(constraints, fallback)? {
            ProjectedConstraintProgress::Complete => Ok(true),
            ProjectedConstraintProgress::Pending => Ok(false),
            ProjectedConstraintProgress::Conflict(error) => {
                let plan = self.checker.signatures.plan()?;
                Err(error.into_legacy_diagnostic(&plan.budget))
            }
        }
    }

    pub(super) fn constrain_projected_terms_progress(
        &mut self,
        constraints: Vec<(InferTerm, InferTerm)>,
        fallback: bool,
    ) -> Result<ProjectedConstraintProgress, String> {
        let mut pending = Vec::new();
        for (formal, actual) in constraints {
            self.split_projected_constraint(formal, actual, &mut pending, 0)?;
        }
        let mut defaults = false;
        while !pending.is_empty() {
            let count = pending.len();
            let mut deferred = Vec::new();
            for (formal, actual) in pending {
                if let Some(formal) = self.normalize_schema_projections(&formal, 0, defaults)? {
                    // Only a type-equation conflict is a candidate mismatch.
                    // Proof/metadata admission errors above and resource
                    // failures here retain their diagnostic category.
                    if let Err(error) = self.core.constrain(&formal, &actual, 0) {
                        let plan = self.checker.signatures.plan()?;
                        if error.kind() == tr::FactsErrorKind::InvalidMetadata {
                            return Ok(ProjectedConstraintProgress::Conflict(error));
                        }
                        return Err(error.into_legacy_diagnostic(&plan.budget));
                    }
                } else {
                    deferred.push((formal, actual));
                }
            }
            if deferred.len() == count {
                if fallback && !defaults {
                    // Numeric defaults are a final phase, after every ordinary
                    // and currently closed projected constraint has settled.
                    defaults = true;
                } else {
                    return Ok(ProjectedConstraintProgress::Pending);
                }
            }
            pending = deferred;
        }
        Ok(ProjectedConstraintProgress::Complete)
    }

    // A projected child must not block an independent sibling from binding the
    // very slot it reads: Result<F, F::Err> first learns F from the expected
    // Result's success type, then normalizes F::Err. Decompose only identical
    // constructors and canonical nominal declarations. This is an inference
    // equation, never evidence for a cast or for projection membership.
    fn split_projected_constraint(
        &self,
        formal: InferTerm,
        actual: InferTerm,
        output: &mut Vec<(InferTerm, InferTerm)>,
        depth: usize,
    ) -> Result<(), String> {
        self.checker.infer_work(depth)?;
        match (formal.as_ref(), actual.as_ref()) {
            (InferKind::PhantomData(a, left), InferKind::PhantomData(b, right)) if a == b => {
                self.split_projected_constraint(left.clone(), right.clone(), output, depth + 1)?;
            }
            (InferKind::FnPointer(left, lo), InferKind::FnPointer(right, ro))
                if left.len() == right.len() =>
            {
                for (left, right) in left
                    .iter()
                    .chain(std::iter::once(lo))
                    .zip(right.iter().chain(std::iter::once(ro)))
                {
                    self.split_projected_constraint(
                        left.clone(),
                        right.clone(),
                        output,
                        depth + 1,
                    )?;
                }
            }
            (InferKind::Tuple(left), InferKind::Tuple(right)) if left.len() == right.len() => {
                for (left, right) in left.iter().zip(right.iter()) {
                    self.split_projected_constraint(
                        left.clone(),
                        right.clone(),
                        output,
                        depth + 1,
                    )?;
                }
            }
            (InferKind::Nominal(left_id, left), InferKind::Nominal(right_id, right))
                if left_id == right_id && left.len() == right.len() =>
            {
                for (left, right) in left.iter().zip(right.iter()) {
                    self.split_projected_constraint(
                        left.clone(),
                        right.clone(),
                        output,
                        depth + 1,
                    )?;
                }
            }
            (InferKind::Ref(left_kind, left), InferKind::Ref(right_kind, right))
            | (InferKind::RawPtr(left_kind, left), InferKind::RawPtr(right_kind, right))
                if left_kind == right_kind =>
            {
                self.split_projected_constraint(left.clone(), right.clone(), output, depth + 1)?;
            }
            (InferKind::Array(left, left_len), InferKind::Array(right, right_len))
                if left_len == right_len =>
            {
                self.split_projected_constraint(left.clone(), right.clone(), output, depth + 1)?;
            }
            (InferKind::Slice(left), InferKind::Slice(right)) => {
                self.split_projected_constraint(left.clone(), right.clone(), output, depth + 1)?;
            }
            _ => {
                self.checker.infer_storage(1)?;
                output.push((formal, actual));
            }
        }
        Ok(())
    }
}

impl Inference<'_, '_> {
    /// `Proj = Rhs`: once `Proj` normalizes from solved slots, its normal
    /// form is evidence for the slots of `Rhs`. Unsolved inputs add nothing.
    pub(super) fn constrain_projection_equality(
        &mut self,
        projection: &ir::Ty,
        rhs: &ir::Ty,
        owner_bindings: Option<&HashMap<ir::ParamId, ir::Ty>>,
        fallback: bool,
    ) -> Result<(), String> {
        let Some(normal) = self.projected_schema(projection, 0, fallback)? else {
            return Ok(());
        };
        // Bind enclosing Self/trait/impl parameters in the inference term,
        // including nominal arguments. Rebuilding a raw nominal Ty would lose
        // its canonical identity or leave the owner's rigid parameters behind.
        let rhs = self.schema(rhs, 0)?;
        let rhs = match owner_bindings {
            Some(bindings) => self.checker.method_owner_term(&rhs, bindings, 0)?,
            None => rhs,
        };
        self.constrain(&rhs, &normal, 0)
    }
}
