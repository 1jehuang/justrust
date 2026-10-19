//! Explicit generic arguments containing `_` participate in ordinary call inference.
use super::*;

impl Checker<'_> {
    pub(super) fn partial_type_term(&self, ty: &Type, depth: usize) -> Result<InferTerm, String> {
        self.infer_work(depth)?;
        if !partial_annotation::has_nested_infer(ty) {
            return self.infer_type_term(&self.checked_type_actual(ty)?, None, depth + 1);
        }
        let term = match ty {
            Type::Infer => InferKind::Unknown(false),
            Type::Ref { kind, referent, .. } => {
                InferKind::Ref(*kind, self.partial_type_term(referent, depth + 1)?)
            }
            Type::RawPtr { kind, pointee } => {
                InferKind::RawPtr(*kind, self.partial_type_term(pointee, depth + 1)?)
            }
            Type::Array(element, len) => {
                InferKind::Array(self.partial_type_term(element, depth + 1)?, *len)
            }
            Type::Tuple(fields) => {
                self.infer_storage(fields.len())?;
                InferKind::Tuple(
                    fields
                        .iter()
                        .map(|field| self.partial_type_term(field, depth + 1))
                        .collect::<Result<Vec<_>, _>>()?
                        .into(),
                )
            }
            Type::Apply(path) => {
                let plan = self.signatures.plan()?;
                let key = self.path_key(&path.path(self.names()?)?, Namespace::Type)?;
                let decl = *plan
                    .registry
                    .nominals
                    .get(&key)
                    .ok_or("unknown nominal type in partial argument")?;
                let (_, args) = path.application()?;
                self.infer_storage(args.len())?;
                InferKind::Nominal(
                    decl,
                    args.iter()
                        .map(|arg| self.partial_type_term(arg, depth + 1))
                        .collect::<Result<Vec<_>, _>>()?
                        .into(),
                )
            }
            _ => return Err("unsupported: partial type form in generic argument".into()),
        };
        self.infer_node(term)
    }

    pub(super) fn resolved_method_actuals(
        &self,
        owner: &ir::Ty,
        name: &AstIdent,
        type_args: &[Type],
        args: &[Expr],
        expected: Option<&ir::Ty>,
    ) -> Result<Vec<ir::Ty>, String> {
        let partial = type_args.iter().any(partial_annotation::has_nested_infer);
        {
            let inferred = if type_args.is_empty() && expected.is_none() {
                self.infer_method_type_args(owner, name, args)?
            } else {
                self.infer_method_type_args_partial(owner, name, args, type_args, expected)?
            };
            if let Some(inferred) = inferred {
                return Ok(inferred);
            }
            if partial {
                return Err("E0282: type annotations needed for method generic arguments".into());
            }
        }
        self.infer_storage(type_args.len())?;
        type_args
            .iter()
            .map(|ty| self.checked_type_actual(ty))
            .collect()
    }

    /// The application path remains canonical and unchanged. Only its `_`
    /// occurrences are solved, with all written concrete constraints retained.
    pub(super) fn partial_call_application(
        &self,
        path: &GenericPath,
        value: &Expr,
        expected: Option<&ir::Ty>,
        fallback: bool,
    ) -> Result<Option<(PathKey, Vec<ir::Ty>)>, String> {
        let (_, args) = path.application()?;
        let partial = args.iter().any(partial_annotation::has_nested_infer);
        let Expr::Call { name, args: values } = value else {
            return if partial {
                Err("unsupported: partial generic application requires a function call".into())
            } else {
                Ok(None)
            };
        };
        let plan = self.signatures.plan()?;
        let key = self.path_key(name, Namespace::Value)?;
        let declaration = *plan
            .registry
            .functions
            .get(&key)
            .ok_or("unresolved function in partial application")?;
        let anonymous = plan
            .registry
            .source_callable(declaration)
            .map_or(0, |f| f.impl_params);
        if !partial && anonymous == 0 {
            return Ok(None);
        }
        plan.registry
            .check_explicit_impl_params(declaration, args.len())?;
        let actuals = self
            .infer_call_args_with_partial(declaration, values, expected, fallback, Some(args))?
            .ok_or("E0282: type annotations needed for partial generic arguments")?;
        Ok(Some((key, actuals)))
    }
}
