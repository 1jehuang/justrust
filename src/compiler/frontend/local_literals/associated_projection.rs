//! Associated projections retain their own structural inputs until those inputs
//! resolve. An unrelated unsolved callable actual cannot authorize or block a
//! closed projection. Only the caller's validated world can normalize one.
use super::*;

#[derive(Clone, Debug)]
pub(super) struct ProjectionTerm {
    pub declaration: ir::DeclId,
    pub item: ir::DeclId,
    pub receiver: Term,
    pub arguments: Vec<Term>,
}

impl<'s> Solver<'_, '_, 's> {
    /// Projection normalization proves a semantic type, not its executable
    /// storage identity. Resolve any source nominal in that normal form through
    /// the same prepared graph reads used by ordinary nominal terms.
    pub(super) fn resolve_projection_representation(
        &self,
        ty: &ir::Ty,
        depth: usize,
    ) -> Result<Option<ir::Ty>, String> {
        self.checker.infer_work(depth)?;
        let Some(inputs) = self.preparation else {
            return Ok(Some(ty.clone()));
        };
        let plan = self.checker.signatures.plan()?;
        self.checker
            .facts()
            .validate_ty_with_budget(ty, &plan.budget)?;
        if let Some((declaration, arguments)) = nominal_parts(ty) {
            let capability = match ty {
                ir::Ty::Struct(value) => value.realization(),
                ir::Ty::Enum(value) => value.realization(),
                _ => None,
            };
            // A typed read is already an exact representation. Never launder
            // another engine's identity through declaration/argument lookup.
            if capability.is_some() {
                return Ok(Some(ty.clone()));
            }
            let demand = self.checker.nominal_input_demand(declaration, arguments)?;
            let Some(realized) = (inputs.nominal_type)(&demand)? else {
                self.record_missing_nominal(demand)?;
                return Ok(None);
            };
            self.checker
                .facts()
                .validate_ty_with_budget(&realized, &plan.budget)?;
            if !matches!((ty, &realized),
                (ir::Ty::Struct(_), ir::Ty::Struct(value)) if value.realization().is_some())
                && !matches!((ty, &realized),
                    (ir::Ty::Enum(_), ir::Ty::Enum(value)) if value.realization().is_some())
            {
                return Err("invalid metadata: projected nominal lacks its graph read".into());
            }
            if !plan
                .trait_world()?
                .types_equal(std::slice::from_ref(ty), std::slice::from_ref(&realized))
                .map_err(|error| error.into_legacy_diagnostic(&plan.budget))?
            {
                return Err("invalid metadata: projected nominal realization changed type".into());
            }
            return Ok(Some(realized));
        }
        Ok(Some(match ty {
            ir::Ty::Tuple(fields) => {
                self.checker.infer_storage(fields.len())?;
                let mut mapped = Vec::new();
                for field in fields.iter() {
                    let Some(field) = self.resolve_projection_representation(field, depth + 1)?
                    else {
                        return Ok(None);
                    };
                    mapped.push(field);
                }
                ir::Ty::Tuple(mapped.into())
            }
            ir::Ty::Ref { kind, referent } => {
                let Some(referent) = self.resolve_projection_representation(referent, depth + 1)?
                else {
                    return Ok(None);
                };
                ir::Ty::Ref {
                    kind: *kind,
                    referent: Arc::new(referent),
                }
            }
            ir::Ty::RawPtr { kind, pointee } => {
                let Some(pointee) = self.resolve_projection_representation(pointee, depth + 1)?
                else {
                    return Ok(None);
                };
                ir::Ty::RawPtr {
                    kind: *kind,
                    pointee: Arc::new(pointee),
                }
            }
            ir::Ty::Array { element, len } => {
                let Some(element) = self.resolve_projection_representation(element, depth + 1)?
                else {
                    return Ok(None);
                };
                ir::Ty::Array {
                    element: Arc::new(element),
                    len: *len,
                }
            }
            ir::Ty::Slice { element } => {
                let Some(element) = self.resolve_projection_representation(element, depth + 1)?
                else {
                    return Ok(None);
                };
                ir::Ty::Slice {
                    element: Arc::new(element),
                }
            }
            _ => ty.clone(),
        }))
    }

    pub(super) fn associated_projection(
        &mut self,
        projection: &ir::AssociatedTypeProjection,
        params: &HashMap<ir::ParamId, Term>,
        depth: usize,
    ) -> Result<Term, String> {
        let receiver = self.schema(&projection.trait_ref.self_ty, params, depth + 1)?;
        let arguments = projection
            .trait_ref
            .args
            .iter()
            .map(|argument| self.schema(argument, params, depth + 1))
            .collect::<Result<Vec<_>, _>>()?;
        if let Some(term) = self.structural_projection(
            receiver,
            projection.trait_ref.declaration,
            projection.item,
            &arguments,
            depth + 1,
        )? {
            return Ok(term);
        }
        self.node(Domain::AssociatedProjection(ProjectionTerm {
            declaration: projection.trait_ref.declaration,
            item: projection.item,
            receiver,
            arguments,
        }))
    }

    fn projection_input_ready(&self, term: Term, depth: usize) -> Result<bool, String> {
        self.checker.infer_work(depth)?;
        Ok(match self.domain(term) {
            Domain::Unknown | Domain::Callable(_) => false,
            Domain::Integer | Domain::Float => self.projection_defaults,
            Domain::Nominal(_, terms) | Domain::Tuple(terms) | Domain::Opaque(_, terms) => {
                for term in terms {
                    if !self.projection_input_ready(term, depth + 1)? {
                        return Ok(false);
                    }
                }
                true
            }
            Domain::Ref(_, term)
            | Domain::RawPtr(_, term)
            | Domain::Array(term, _)
            | Domain::Slice(term) => self.projection_input_ready(term, depth + 1)?,
            Domain::AssociatedProjection(projection) => {
                if !self.projection_input_ready(projection.receiver, depth + 1)? {
                    return Ok(false);
                }
                for term in projection.arguments {
                    if !self.projection_input_ready(term, depth + 1)? {
                        return Ok(false);
                    }
                }
                true
            }
            Domain::Fixed(_) => true,
        })
    }

    fn projection_input_symbolic(&self, term: Term, depth: usize) -> Result<bool, String> {
        self.checker.infer_work(depth)?;
        Ok(match self.domain(term) {
            Domain::Fixed(ir::Ty::Param(_) | ir::Ty::AssociatedType(_)) => true,
            Domain::Nominal(_, terms) | Domain::Tuple(terms) | Domain::Opaque(_, terms) => {
                for term in terms {
                    if self.projection_input_symbolic(term, depth + 1)? {
                        return Ok(true);
                    }
                }
                false
            }
            Domain::Ref(_, term)
            | Domain::RawPtr(_, term)
            | Domain::Array(term, _)
            | Domain::Slice(term) => self.projection_input_symbolic(term, depth + 1)?,
            Domain::AssociatedProjection(projection) => {
                if self.projection_input_symbolic(projection.receiver, depth + 1)? {
                    return Ok(true);
                }
                for term in projection.arguments {
                    if self.projection_input_symbolic(term, depth + 1)? {
                        return Ok(true);
                    }
                }
                false
            }
            _ => false,
        })
    }

    pub(super) fn resolve_associated_projection(
        &self,
        projection: &ProjectionTerm,
        depth: usize,
    ) -> Result<Option<ir::Ty>, String> {
        if !self.projection_input_ready(projection.receiver, depth + 1)? {
            return Ok(None);
        }
        let Some(receiver) = self.resolve(projection.receiver, depth + 1)? else {
            return Ok(None);
        };
        let mut arguments = Vec::new();
        for term in &projection.arguments {
            if !self.projection_input_ready(*term, depth + 1)? {
                return Ok(None);
            }
            let Some(argument) = self.resolve(*term, depth + 1)? else {
                return Ok(None);
            };
            arguments.push(argument);
        }
        let mut symbolic = self.projection_input_symbolic(projection.receiver, depth + 1)?;
        for term in &projection.arguments {
            symbolic |= self.projection_input_symbolic(*term, depth + 1)?;
        }
        let plan = self.checker.signatures.plan()?;
        let caller = self.checker.facts();
        let projection = ir::Ty::AssociatedType(Arc::new(ir::AssociatedTypeProjection {
            trait_ref: ir::TraitRef {
                declaration: projection.declaration,
                self_ty: receiver,
                args: arguments.into(),
            },
            item: projection.item,
        }));
        if symbolic {
            caller.validate_ty_metadata(&projection)?;
            if let Some(facts) = caller.validated()
                && let ir::Ty::AssociatedType(projected) = &projection
            {
                for predicate in facts.predicates().predicates() {
                    plan.budget.charge_work(1)?;
                    if let ir::Predicate::ProjectionEq {
                        projection: declared,
                        ..
                    } = predicate
                        && facts
                            .world()
                            .same_projection(declared, projected)
                            .map_err(|error| error.into_legacy_diagnostic(&plan.budget))?
                    {
                        let equality = facts
                            .world()
                            .normalize_projection_equality(projected, facts)
                            .map_err(|error| error.into_legacy_diagnostic(&plan.budget))?;
                        return Ok(Some(equality.ty().clone()));
                    }
                }
                return self
                    .projection_metadata
                    .as_ref()
                    .ok_or_else(|| "missing original solver metadata scope".to_string())?
                    .normalize(facts, &projection, &plan.budget)
                    .map(Some);
            }
            return Ok(Some(projection));
        }
        let Some(facts) = caller.validated() else {
            // Universal structural metadata is not executable normalization proof.
            caller.validate_ty_with_budget(&projection, &plan.budget)?;
            return Ok(Some(projection));
        };
        let normal = self
            .projection_metadata
            .as_ref()
            .ok_or_else(|| "missing original solver metadata scope".to_string())?
            .normalize(facts, &projection, &plan.budget)?;
        caller.validate_ty_with_budget(&normal, &plan.budget)?;
        Ok(Some(normal))
    }

    pub(super) fn reduce_associated_projection(&mut self, term: Term) -> Result<bool, String> {
        let root = self.root(term);
        let Domain::AssociatedProjection(projection) = self.domain(root) else {
            return Ok(false);
        };
        let Some(normal) = self.resolve_associated_projection(&projection, 0)? else {
            return Ok(false);
        };
        // A symbolic caller-owned projection can remain opaque. Do not re-enter
        // schema reconstruction indefinitely or claim equality to a concrete type.
        let resolved = if matches!(normal, ir::Ty::AssociatedType(_)) {
            self.node(Domain::Fixed(normal))?
        } else {
            self.fixed(normal)?
        };
        self.variables[root].domain = self.domain(resolved);
        Ok(true)
    }

    /// Reverse an associated equation only across a complete family of closed
    /// nominal impls. No branch is chosen if an open impl or a second matching
    /// output could apply. This is an inference hint, never a trait proof.
    fn infer_closed_projection_input(&mut self, left: Term, right: Term) -> Result<bool, String> {
        let Domain::AssociatedProjection(projection) = self.domain(left) else {
            return Ok(false);
        };
        let Domain::Nominal(nominal, _) = self.domain(projection.receiver) else {
            return Ok(false);
        };
        if !self.numeric_nominal_actuals(projection.receiver)?
            || !projection.arguments.is_empty()
            || self.projection_input_ready(projection.receiver, 0)?
            || !self.projection_input_ready(right, 0)?
        {
            return Ok(false);
        }
        let Some(expected) = self.resolve(right, 0)? else {
            return Ok(false);
        };
        let plan = self.checker.signatures.plan()?;
        let world = plan.trait_world()?;
        let mut selected = None;
        for header in world.impl_headers() {
            self.checker.infer_work(0)?;
            if header.trait_ref.declaration != projection.declaration {
                continue;
            }
            let Some((owner, _)) = nominal_parts(&header.trait_ref.self_ty) else {
                if matches!(header.trait_ref.self_ty, ir::Ty::Param(_)) {
                    return Ok(false);
                }
                continue;
            };
            if owner != nominal {
                continue;
            }
            if header.schema_env.is_generic() {
                return Ok(false);
            }
            let Some(value) = header
                .associated_types
                .iter()
                .find(|v| v.associated == projection.item)
            else {
                continue;
            };
            if value.rhs == expected && selected.replace(header.trait_ref.self_ty.clone()).is_some()
            {
                return Ok(false);
            }
        }
        let Some(receiver) = selected else {
            return Ok(false);
        };
        let receiver = self.fixed(receiver)?;
        self.unify(projection.receiver, receiver)?;
        Ok(true)
    }

    pub(super) fn solve_associated_equalities(&mut self) -> Result<bool, String> {
        let pending = std::mem::take(&mut self.associated_equalities);
        let mut progress = false;
        for (left, right) in pending {
            progress |= self.infer_closed_projection_input(left, right)?;
            progress |= self.infer_closed_projection_input(right, left)?;
            progress |= self.reduce_associated_projection(left)?;
            progress |= self.reduce_associated_projection(right)?;
            if matches!(self.domain(left), Domain::AssociatedProjection(_))
                || matches!(self.domain(right), Domain::AssociatedProjection(_))
            {
                self.associated_equalities.push((left, right));
            } else {
                self.unify(left, right)?;
                progress = true;
            }
        }
        Ok(progress)
    }
}
