//! Executable source ingress uses the same borrowed graph-read inputs as hints.
//! Recipes are descriptions only. Ordinary source, region and executable checks
//! remain required; no missing read may fall back to the raw nominal interner.
use super::*;

#[cfg(test)]
mod authority_tests;
pub(in crate::compiler::frontend) mod parent_local;

impl Checker<'_> {
    /// Representation tokens authenticate graph reads and source slots separately.
    /// Only after replaying both may a source consumer compare semantic types.
    pub(super) fn nominal_input_types_equal(
        &self,
        left: &ir::Ty,
        right: &ir::Ty,
    ) -> Result<bool, String> {
        if self.nominal_inputs.is_none() {
            return Ok(false);
        }
        let plan = self.signatures.plan()?;
        self.facts()
            .validate_ty_metadata_with_budget(left, &plan.budget)?;
        self.facts()
            .validate_ty_metadata_with_budget(right, &plan.budget)?;
        plan.trait_world()?
            .types_equal(std::slice::from_ref(left), std::slice::from_ref(right))
            .map_err(|error| error.into_legacy_diagnostic(&plan.budget))
    }

    pub(super) fn source_type_actual(&self, source: &Type) -> Result<ir::Ty, String> {
        let Some(inputs) = self.nominal_inputs else {
            return checked_ty(source, self.signatures);
        };
        let shape = (inputs.source_type)(source)?
            .ok_or("unsupported: generic actual source recipe is not available")?;
        let mut missing = Vec::new();
        if let Some(ty) = recipe_type(self, inputs, &shape, &mut missing, 0)? {
            self.facts()
                .validate_ty_metadata_with_budget(&ty, &self.signatures.plan()?.budget)?;
            return Ok(ty);
        }
        *self.nominal_blocked.borrow_mut() = Some(NominalPreparation::Blocked {
            nominals: missing.into(),
            declarations: Arc::from([]),
            needs_source_schemas: false,
            unresolved_constraints: false,
        });
        Err("missing graph input: generic actual nominal realization".into())
    }

    pub(super) fn source_local_ty(&mut self, source: &Type) -> Result<ir::Ty, String> {
        let Some(inputs) = self.nominal_inputs else {
            return checked_local_ty(source, self.signatures);
        };
        // Preserve the existing local named-reference annotation boundary before
        // requesting a region-erased description from the authenticated adapter.
        let mut inner = source;
        for _ in 0..source_reference_layers(source)? {
            if let Type::Ref {
                region, referent, ..
            } = inner
            {
                if region.is_some() {
                    return Err("named lifetime annotations on locals are unsupported; use an elided local reference annotation".into());
                }
                inner = referent;
            }
        }
        let shape = (inputs.source_type)(source)?
            .ok_or("unsupported: local annotation source recipe is not available")?;
        let mut missing = Vec::new();
        let result = recipe_type(self, inputs, &shape, &mut missing, 0)?;
        if let Some(ty) = result {
            return Ok(ty);
        }
        *self.nominal_blocked.borrow_mut() = Some(NominalPreparation::Blocked {
            nominals: missing.into(),
            declarations: Arc::from([]),
            needs_source_schemas: false,
            unresolved_constraints: false,
        });
        Err("missing graph input: local annotation nominal realization".into())
    }

    pub(super) fn materialize_nominal_input(
        &self,
        declaration: ir::DeclId,
        actuals: &[ir::Ty],
    ) -> Result<ir::Ty, String> {
        let plan = self.signatures.plan()?;
        let Some(inputs) = self.nominal_inputs else {
            return plan.intern_nominal(declaration, actuals, self.facts(), &plan.budget);
        };
        if parent_local::contains_closure(actuals, &plan.budget, 0)? {
            return self.mint_parent_local_nominal(declaration, actuals);
        }
        let demand = self.nominal_input_demand(declaration, actuals)?;
        if let Some(ty) = (inputs.nominal_type)(&demand)? {
            return Ok(ty);
        }
        self.infer_storage(1)?;
        *self.nominal_blocked.borrow_mut() = Some(NominalPreparation::Blocked {
            nominals: Arc::from([demand]),
            declarations: Arc::from([]),
            needs_source_schemas: false,
            unresolved_constraints: false,
        });
        Err("missing graph input: constructor nominal realization".into())
    }

    /// Convert a canonical source constructor through its complete graph receipt.
    /// This is not type equality: representation identities remain distinct.
    pub(super) fn realize_constructor_source(&self, source: &ir::Ty) -> Result<ir::Ty, String> {
        if self.nominal_inputs.is_none() {
            return Ok(source.clone());
        }
        let Some((declaration, arguments)) = nominal_parts(source) else {
            return Ok(source.clone());
        };
        let plan = self.signatures.plan()?;
        let budget = &plan.budget;
        self.facts()
            .validate_ty_metadata_with_budget(source, budget)?;
        let source_identity = match source {
            ir::Ty::Struct(value) => value.id(),
            ir::Ty::Enum(value) => value.id(),
            _ => return Err("invalid metadata: constructor source is not nominal".into()),
        };
        if matches!(source_identity.index, ir::NominalIndex::GraphRealization(_)) {
            // An existing graph read is not a raw source template. Preserve its
            // exact typed identity, including engine, rather than laundering it
            // through declaration/argument lookup into a different engine's read.
            return self
                .realized_constructor_expectation(declaration, Some(source))?
                .ok_or_else(|| {
                    "invalid metadata: graph constructor realization is missing".into()
                });
        }
        if parent_local::contains_closure(arguments, budget, 0)? {
            return self.mint_parent_local_nominal(declaration, arguments);
        }
        let realized = self.materialize_nominal_input(declaration, arguments)?;
        let Some((owner, actuals)) = nominal_parts(&realized) else {
            return Err("invalid metadata: constructor realization is not nominal".into());
        };
        if owner != declaration || actuals.len() != arguments.len() {
            return Err("invalid metadata: constructor realization owner or arity changed".into());
        }
        for (source, actual) in arguments.iter().zip(actuals) {
            self.infer_work(0)?;
            let source = self.realize_constructor_actual(source, 0)?;
            if !replay_type_equal(&source, actual, budget)
                .map_err(|e| e.into_legacy_diagnostic(budget))?
            {
                return Err(
                    "invalid metadata: constructor realization ordered actuals changed".into(),
                );
            }
        }
        let (_, template) = plan.nominal_schema(declaration, budget)?;
        let (template_id, capability) = match (&template, &realized) {
            (ir::Ty::Struct(template), ir::Ty::Struct(value)) => {
                (template.id(), value.realization())
            }
            (ir::Ty::Enum(template), ir::Ty::Enum(value)) => (template.id(), value.realization()),
            _ => {
                return Err(
                    "invalid metadata: constructor realization changed nominal kind".into(),
                );
            }
        };
        let capability =
            capability.ok_or("invalid metadata: constructor realization lacks typed read")?;
        if capability.source_application() != template_id {
            return Err(
                "invalid metadata: constructor realization source application changed".into(),
            );
        }
        // This replays the exact source schema, closed ordered substitution, key,
        // and every source-field/normalized-field receipt, not only the shell.
        let world = plan.trait_world()?;
        capability
            .replay(&realized, &world)
            .map_err(|e| e.to_string())?;
        self.facts()
            .validate_ty_metadata_with_budget(&realized, budget)?;
        Ok(realized)
    }

    /// Transport only source representations. Existing graph identities must
    /// survive exact replay, including nested identities from another engine.
    pub(super) fn realize_constructor_actual(
        &self,
        ty: &ir::Ty,
        depth: usize,
    ) -> Result<ir::Ty, String> {
        self.infer_work(depth)?;
        Ok(match ty {
            ir::Ty::Struct(_) | ir::Ty::Enum(_) => self.realize_constructor_source(ty)?,
            ir::Ty::Tuple(fields) => {
                self.infer_storage(fields.len())?;
                ir::Ty::Tuple(
                    fields
                        .iter()
                        .map(|field| self.realize_constructor_actual(field, depth + 1))
                        .collect::<Result<Vec<_>, _>>()?
                        .into(),
                )
            }
            ir::Ty::Ref { kind, referent } => ir::Ty::Ref {
                kind: *kind,
                referent: Arc::new(self.realize_constructor_actual(referent, depth + 1)?),
            },
            ir::Ty::RawPtr { kind, pointee } => ir::Ty::RawPtr {
                kind: *kind,
                pointee: Arc::new(self.realize_constructor_actual(pointee, depth + 1)?),
            },
            ir::Ty::Array { element, len } => ir::Ty::Array {
                element: Arc::new(self.realize_constructor_actual(element, depth + 1)?),
                len: *len,
            },
            ir::Ty::Slice { element } => ir::Ty::Slice {
                element: Arc::new(self.realize_constructor_actual(element, depth + 1)?),
            },
            _ => ty.clone(),
        })
    }

    pub(super) fn realized_constructor_expectation(
        &self,
        declaration: ir::DeclId,
        expected: Option<&ir::Ty>,
    ) -> Result<Option<ir::Ty>, String> {
        if self.nominal_inputs.is_none() {
            return Ok(None);
        }
        let Some(expected) = expected else {
            return Ok(None);
        };
        let Some((owner, arguments)) = nominal_parts(expected) else {
            return Ok(None);
        };
        if owner != declaration {
            return Ok(None);
        }
        let realized = self.materialize_nominal_input(declaration, arguments)?;
        let Some((actual_owner, actual_arguments)) = nominal_parts(&realized) else {
            return Err("invalid metadata: constructor callback returned non-nominal type".into());
        };
        if actual_owner != declaration || arguments.len() != actual_arguments.len() {
            return Err("invalid metadata: constructor callback owner or arity drift".into());
        }
        let budget = &self.signatures.plan()?.budget;
        for (expected, actual) in arguments.iter().zip(actual_arguments) {
            self.infer_work(0)?;
            if !replay_type_equal(expected, actual, budget)
                .map_err(|e| e.into_legacy_diagnostic(budget))?
            {
                return Err(
                    "invalid metadata: constructor callback ordered arguments drift".into(),
                );
            }
        }
        if !replay_type_equal(expected, &realized, budget)
            .map_err(|e| e.into_legacy_diagnostic(budget))?
        {
            return Err("invalid metadata: constructor expected realization mismatch".into());
        }
        Ok(Some(realized))
    }

    pub(super) fn nominal_input_demand(
        &self,
        declaration: ir::DeclId,
        actuals: &[ir::Ty],
    ) -> Result<NominalDemand, String> {
        self.infer_storage(actuals.len())?;
        let arguments = actuals
            .iter()
            .map(|ty| type_shape(self, ty, 0))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(NominalDemand {
            declaration,
            arguments: arguments.into(),
        })
    }
}

fn type_shape(checker: &Checker<'_>, ty: &ir::Ty, depth: usize) -> Result<NominalShape, String> {
    checker.infer_work(depth)?;
    let children = |types: &[ir::Ty]| -> Result<Arc<[NominalShape]>, String> {
        checker.infer_storage(types.len())?;
        types
            .iter()
            .map(|ty| type_shape(checker, ty, depth + 1))
            .collect::<Result<Vec<_>, _>>()
            .map(Arc::from)
    };
    Ok(match ty {
        ir::Ty::Struct(n) => NominalShape::Nominal(n.def().source_decl, children(&n.def().type_args)?),
        ir::Ty::Enum(n) => NominalShape::Nominal(n.def().source_decl, children(&n.def().type_args)?),
        ir::Ty::Tuple(fields) => NominalShape::Tuple(children(fields)?),
        ir::Ty::Ref { kind, referent } => NominalShape::Ref(*kind, Arc::new(type_shape(checker, referent, depth + 1)?)),
        ir::Ty::RawPtr { kind, pointee } => NominalShape::RawPtr(*kind, Arc::new(type_shape(checker, pointee, depth + 1)?)),
        ir::Ty::Array { element, len } => NominalShape::Array(Arc::new(type_shape(checker, element, depth + 1)?), *len),
        ir::Ty::Slice { element } => NominalShape::Slice(Arc::new(type_shape(checker, element, depth + 1)?)),
        ir::Ty::AssociatedType(p) => NominalShape::Projection {
            declaration: p.trait_ref.declaration, item: p.item,
            receiver: Arc::new(type_shape(checker, &p.trait_ref.self_ty, depth + 1)?),
            arguments: children(&p.trait_ref.args)?,
        },
        ir::Ty::Dyn(object) if object.is_closed_leaf() => NominalShape::Leaf(ty.clone()),
        ir::Ty::Closure(_) | ir::Ty::Dyn(_) => return Err("unsupported: nominal ingress requires independently authenticated closure or dynamic metadata".into()),
        leaf => NominalShape::Leaf(leaf.clone()),
    })
}

fn recipe_type(
    checker: &Checker<'_>,
    inputs: &NominalPreparationInputs<'_>,
    shape: &NominalShape,
    missing: &mut Vec<NominalDemand>,
    depth: usize,
) -> Result<Option<ir::Ty>, String> {
    checker.infer_work(depth)?;
    Ok(Some(match shape {
        NominalShape::Leaf(ty) => match ty {
            ir::Ty::Unit
            | ir::Ty::Never
            | ir::Ty::Bool
            | ir::Ty::Char
            | ir::Ty::Str
            | ir::Ty::Int(_)
            | ir::Ty::Float(_)
            | ir::Ty::Param(_) => ty.clone(),
            ir::Ty::Dyn(object) if object.is_closed_leaf() => {
                ty.clone()
            }
            _ => return Err("invalid metadata: non-leaf nominal ingress recipe".into()),
        },
        NominalShape::Nominal(declaration, arguments) => {
            let demand = NominalDemand {
                declaration: *declaration,
                arguments: arguments.clone(),
            };
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
        NominalShape::Tuple(fields) => {
            checker.infer_storage(fields.len())?;
            let mut result = Vec::new();
            let mut complete = true;
            for field in fields.iter() {
                match recipe_type(checker, inputs, field, missing, depth + 1)? {
                    Some(ty) => result.push(ty),
                    None => complete = false,
                }
            }
            if !complete {
                return Ok(None);
            }
            ir::Ty::Tuple(result.into())
        }
        NominalShape::Ref(kind, inner) => {
            let Some(referent) = recipe_type(checker, inputs, inner, missing, depth + 1)? else {
                return Ok(None);
            };
            ir::Ty::Ref {
                kind: *kind,
                referent: Arc::new(referent),
            }
        }
        NominalShape::RawPtr(kind, inner) => {
            let Some(pointee) = recipe_type(checker, inputs, inner, missing, depth + 1)? else {
                return Ok(None);
            };
            ir::Ty::RawPtr {
                kind: *kind,
                pointee: Arc::new(pointee),
            }
        }
        NominalShape::Array(inner, len) => {
            let Some(element) = recipe_type(checker, inputs, inner, missing, depth + 1)? else {
                return Ok(None);
            };
            ir::Ty::Array {
                element: Arc::new(element),
                len: *len,
            }
        }
        NominalShape::Slice(inner) => {
            let Some(element) = recipe_type(checker, inputs, inner, missing, depth + 1)? else {
                return Ok(None);
            };
            ir::Ty::Slice {
                element: Arc::new(element),
            }
        }
        NominalShape::Projection { .. } => {
            return Err(
                "unsupported: executable local projection recipe requires sealed normalization"
                    .into(),
            );
        }
    }))
}
