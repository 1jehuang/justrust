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
