//! Constructor expectation provenance, keyed by canonical nominal declarations.
use super::*;

impl Checker<'_> {
    pub(super) fn constructor_field_schema(
        &self,
        declaration: ir::DeclId,
        variant: Option<usize>,
        field: usize,
        expectation: Expectation<'_>,
    ) -> Result<InferTerm, String> {
        let plan = self.signatures.plan()?;
        plan.budget.charge_work(1)?;
        let (scope, nominal) = plan.nominal_schema(declaration, &plan.budget)?;
        let fields = match (&nominal, variant) {
            (ir::Ty::Struct(ty), None) => &ty.def().fields,
            (ir::Ty::Enum(ty), Some(index)) => {
                &ty.def()
                    .variants
                    .get(index)
                    .ok_or("unknown constructor variant schema")?
                    .fields
            }
            _ => return Err("constructor source schema kind mismatch".into()),
        };
        let field_index = field;
        let field = fields
            .get(field_index)
            .ok_or("unknown constructor field schema")?;
        let expected = match expectation {
            Expectation::HasType(ty) => Some(ty),
            _ => None,
        };
        if let Some(realized) = self.realized_constructor_expectation(declaration, expected)? {
            let concrete_fields = match (&realized, variant) {
                (ir::Ty::Struct(ty), None) => &ty.def().fields,
                (ir::Ty::Enum(ty), Some(index)) => {
                    &ty.def()
                        .variants
                        .get(index)
                        .ok_or("unknown realized constructor variant")?
                        .fields
                }
                _ => return Err("realized constructor source kind mismatch".into()),
            };
            if concrete_fields.len() != fields.len() {
                return Err("invalid metadata: realized constructor field topology drift".into());
            }
            let concrete = concrete_fields
                .get(field_index)
                .ok_or("unknown realized constructor field")?;
            return self.infer_type_term(&concrete.ty, None, 0);
        }
        let term = self.infer_type_term(&field.ty, Some(scope.binder()), 0)?;
        let arguments = self.constructor_source_arguments(declaration, expectation, 0)?;
        if let Some(arguments) = &arguments
            && arguments.len() != scope.arity()
        {
            return Err("constructor source argument arity mismatch".into());
        }
        self.replace_constructor_slots(&term, arguments.as_deref(), 0)
    }

    fn constructor_source_arguments(
        &self,
        declaration: ir::DeclId,
        expectation: Expectation<'_>,
        depth: usize,
    ) -> Result<Option<Vec<InferTerm>>, String> {
        self.infer_work(depth)?;
        let term = match expectation {
            Expectation::Application(application) => {
                let inherited =
                    self.constructor_source_arguments(declaration, application.outer, depth + 1)?;
                let mut arguments = Vec::with_capacity(application.arguments.len());
                self.infer_storage(application.arguments.len())?;
                for (index, source) in application.arguments.iter().enumerate() {
                    let written = self.partial_type_term(source, depth + 1)?;
                    arguments.push(self.merge_constructor_context(
                        &written,
                        inherited.as_ref().and_then(|args| args.get(index)),
                        depth + 1,
                    )?);
                }
                return Ok(Some(arguments));
            }
            Expectation::HasType(ty) => self.infer_type_term(ty, None, depth + 1)?,
            Expectation::SourceSchema(_, term) => term.clone(),
            _ => return Ok(None),
        };
        match term.as_ref() {
            InferKind::Nominal(owner, arguments) if *owner == declaration => {
                self.infer_storage(arguments.len())?;
                Ok(Some(arguments.to_vec()))
            }
            _ => Ok(None),
        }
    }

    fn replace_constructor_slots(
        &self,
        term: &InferTerm,
        arguments: Option<&[InferTerm]>,
        depth: usize,
    ) -> Result<InferTerm, String> {
        self.infer_work(depth)?;
        let kind = match term.as_ref() {
            InferKind::Slot(index) => {
                return match arguments.and_then(|args| args.get(*index)) {
                    Some(argument) => Ok(argument.clone()),
                    None => self.infer_node(InferKind::Unknown(false)),
                };
            }
            InferKind::Ref(kind, inner) => InferKind::Ref(
                *kind,
                self.replace_constructor_slots(inner, arguments, depth + 1)?,
            ),
            InferKind::RawPtr(kind, inner) => InferKind::RawPtr(
                *kind,
                self.replace_constructor_slots(inner, arguments, depth + 1)?,
            ),
            InferKind::Array(inner, len) => InferKind::Array(
                self.replace_constructor_slots(inner, arguments, depth + 1)?,
                *len,
            ),
            InferKind::Slice(inner) => {
                InferKind::Slice(self.replace_constructor_slots(inner, arguments, depth + 1)?)
            }
            InferKind::Tuple(fields) => {
                self.infer_storage(fields.len())?;
                InferKind::Tuple(
                    fields
                        .iter()
                        .map(|field| self.replace_constructor_slots(field, arguments, depth + 1))
                        .collect::<Result<Vec<_>, _>>()?
                        .into(),
                )
            }
            InferKind::Nominal(owner, fields) => {
                self.infer_storage(fields.len())?;
                InferKind::Nominal(
                    *owner,
                    fields
                        .iter()
                        .map(|field| self.replace_constructor_slots(field, arguments, depth + 1))
                        .collect::<Result<Vec<_>, _>>()?
                        .into(),
                )
            }
            _ => return Ok(term.clone()),
        };
        self.infer_node(kind)
    }

    fn merge_constructor_context(
        &self,
        written: &InferTerm,
        inherited: Option<&InferTerm>,
        depth: usize,
    ) -> Result<InferTerm, String> {
        self.infer_work(depth)?;
        let Some(inherited) = inherited else {
            return Ok(written.clone());
        };
        match (written.as_ref(), inherited.as_ref()) {
            (InferKind::Unknown(_), _) => Ok(inherited.clone()),
            (InferKind::Nominal(owner, fields), InferKind::Nominal(other, values))
                if owner == other && fields.len() == values.len() =>
            {
                self.infer_storage(fields.len())?;
                let fields = fields
                    .iter()
                    .zip(values.iter())
                    .map(|(written, inherited)| {
                        self.merge_constructor_context(written, Some(inherited), depth + 1)
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                self.infer_node(InferKind::Nominal(*owner, fields.into()))
            }
            (InferKind::Tuple(fields), InferKind::Tuple(values))
                if fields.len() == values.len() =>
            {
                self.infer_storage(fields.len())?;
                let fields = fields
                    .iter()
                    .zip(values.iter())
                    .map(|(written, inherited)| {
                        self.merge_constructor_context(written, Some(inherited), depth + 1)
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                self.infer_node(InferKind::Tuple(fields.into()))
            }
            (InferKind::Array(inner, len), InferKind::Array(value, actual)) if len == actual => {
                let inner = self.merge_constructor_context(inner, Some(value), depth + 1)?;
                self.infer_node(InferKind::Array(inner, *len))
            }
            (InferKind::Slice(inner), InferKind::Slice(value)) => {
                let inner = self.merge_constructor_context(inner, Some(value), depth + 1)?;
                self.infer_node(InferKind::Slice(inner))
            }
            // Reference shape is already firmly written, regardless of holes
            // below the referent. No reference-value move occurs inside it.
            _ => Ok(written.clone()),
        }
    }
}
