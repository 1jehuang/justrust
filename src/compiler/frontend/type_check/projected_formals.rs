//! Resolve only the generic slots read by an associated projection. An
//! unrelated unresolved slot never supplies fabricated normalization evidence.
use super::*;

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
            InferKind::Tuple(fields) | InferKind::Nominal(_, fields) => {
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
            InferKind::Tuple(fields) | InferKind::Nominal(_, fields) => {
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
        let mut pending = constraints;
        let mut defaults = false;
        while !pending.is_empty() {
            let count = pending.len();
            let mut deferred = Vec::new();
            for (formal, actual) in pending {
                if let Some(formal) = self.projected_schema(formal, 0, defaults)? {
                    self.constrain(&formal, &actual, 0)?;
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
                    return Ok(false);
                }
            }
            pending = deferred;
        }
        Ok(true)
    }
}
