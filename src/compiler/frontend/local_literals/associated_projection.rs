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
            Domain::Nominal(_, terms) | Domain::Tuple(terms) => {
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
            Domain::Nominal(_, terms) | Domain::Tuple(terms) => {
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
            return Ok(Some(projection));
        }
        let Some(facts) = caller.validated() else {
            // Universal structural metadata is not executable normalization proof.
            caller.validate_ty_with_budget(&projection, &plan.budget)?;
            return Ok(Some(projection));
        };
        let normal = facts
            .normalize_substituted_projection_type(&projection, &plan.budget)
            .map_err(|error| error.into_legacy_diagnostic(&plan.budget))?;
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

    pub(super) fn solve_associated_equalities(&mut self) -> Result<bool, String> {
        let pending = std::mem::take(&mut self.associated_equalities);
        let mut progress = false;
        for (left, right) in pending {
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
