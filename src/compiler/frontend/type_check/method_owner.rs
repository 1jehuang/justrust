//! Substitute canonical enclosing-impl bindings in nonexecuting inference terms.
use super::*;

impl Checker<'_> {
    pub(super) fn method_owner_term(
        &self,
        term: &InferTerm,
        bindings: &HashMap<ir::ParamId, ir::Ty>,
        depth: usize,
    ) -> Result<InferTerm, String> {
        self.infer_work(depth)?;
        let kind = match term.as_ref() {
            InferKind::Rigid(ir::Ty::Param(parameter)) => {
                return match bindings.get(parameter) {
                    Some(actual) => self.infer_type_term(actual, None, depth + 1),
                    None => Ok(term.clone()),
                };
            }
            InferKind::Rigid(ty @ ir::Ty::AssociatedType(_)) => {
                let bound = super::trait_method_hint::bind_owner(
                    ty,
                    bindings,
                    &self.signatures.plan()?.budget,
                )?;
                if bound == *ty {
                    return Ok(term.clone());
                }
                let plan = self.signatures.plan()?;
                let caller = self.facts();
                let facts = caller
                    .validated()
                    .ok_or("projected method inference requires validated caller facts")?;
                let normal = facts
                    .normalize_substituted_projection_type(&bound, &plan.budget)
                    .map_err(|error| error.into_legacy_diagnostic(&plan.budget))?;
                return self.infer_type_term(&normal, None, depth + 1);
            }
            InferKind::PhantomData(declaration, argument) => InferKind::PhantomData(
                *declaration,
                self.method_owner_term(argument, bindings, depth + 1)?,
            ),
            InferKind::ManuallyDrop(declaration, argument) => InferKind::ManuallyDrop(
                *declaration,
                self.method_owner_term(argument, bindings, depth + 1)?,
            ),
            InferKind::Cell(declaration, argument) => InferKind::Cell(
                *declaration,
                self.method_owner_term(argument, bindings, depth + 1)?,
            ),
            InferKind::FnPointer(inputs, output) => {
                let budget = &self.signatures.plan()?.budget;
                let mut mapped = ir::FnPointerTy::storage(inputs.len(), budget)?;
                for t in inputs.iter() {
                    mapped.push(self.method_owner_term(t, bindings, depth + 1)?);
                }
                InferKind::FnPointer(
                    ir::FnPointerTy::slice(mapped, budget)?,
                    self.method_owner_term(output, bindings, depth + 1)?,
                )
            }
            InferKind::Tuple(fields) => {
                self.infer_storage(fields.len())?;
                InferKind::Tuple(
                    fields
                        .iter()
                        .map(|field| self.method_owner_term(field, bindings, depth + 1))
                        .collect::<Result<Vec<_>, _>>()?
                        .into(),
                )
            }
            InferKind::Nominal(declaration, args) => {
                self.infer_storage(args.len())?;
                InferKind::Nominal(
                    *declaration,
                    args.iter()
                        .map(|arg| self.method_owner_term(arg, bindings, depth + 1))
                        .collect::<Result<Vec<_>, _>>()?
                        .into(),
                )
            }
            InferKind::Opaque(descriptor, args) => {
                self.infer_storage(args.len())?;
                InferKind::Opaque(
                    descriptor.clone(),
                    args.iter()
                        .map(|arg| self.method_owner_term(arg, bindings, depth + 1))
                        .collect::<Result<Vec<_>, _>>()?
                        .into(),
                )
            }
            InferKind::Projection(declaration, item, children) => {
                self.infer_storage(children.len())?;
                InferKind::Projection(
                    *declaration,
                    *item,
                    children
                        .iter()
                        .map(|child| self.method_owner_term(child, bindings, depth + 1))
                        .collect::<Result<Vec<_>, _>>()?
                        .into(),
                )
            }
            InferKind::Ref(kind, referent) => InferKind::Ref(
                *kind,
                self.method_owner_term(referent, bindings, depth + 1)?,
            ),
            InferKind::RawPtr(kind, pointee) => {
                InferKind::RawPtr(*kind, self.method_owner_term(pointee, bindings, depth + 1)?)
            }
            InferKind::Array(element, len) => {
                InferKind::Array(self.method_owner_term(element, bindings, depth + 1)?, *len)
            }
            InferKind::Slice(element) => {
                InferKind::Slice(self.method_owner_term(element, bindings, depth + 1)?)
            }
            _ => return Ok(term.clone()),
        };
        self.infer_node(kind)
    }
}
