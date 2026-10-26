//! Recursive input evidence rebuilt from immutable source and lexical occurrences.
//! Checked types are comparison targets, never an inference source.
use super::*;

pub(super) struct Replay<'a> {
    pub original: &'a OriginalSource,
    pub inventory: &'a SourceInventory,
    pub checked: &'a ir::CheckedFunction,
    pub signatures: &'a Signatures,
    pub budget: &'a ir::PlanBudget,
}
impl Replay<'_> {
    fn equal(&self, a: &ir::Ty, b: &ir::Ty) -> Result<bool, FactsError> {
        self.original
            .facts
            .validate_ty_metadata_with_budget(a, self.budget)?;
        self.original
            .facts
            .validate_ty_metadata_with_budget(b, self.budget)?;
        sr::replay_types_equal(a, b, self.budget)
    }
    pub fn binding(&self, index: usize, depth: usize) -> Result<(ir::LocalId, ir::Ty), FactsError> {
        sr::depth(self.budget, depth)?;
        sr::work(self.budget, 1)?;
        let site = self
            .original
            .bindings
            .get(index)
            .ok_or_else(|| FactsError::invalid(self.budget, "input binding missing"))?;
        validate_original_occurrence(self.original, &site.node, self.budget)?;
        if let Some(parameter) = site.parameter {
            let (local, ty) = source_binding_type(
                self.original,
                index,
                self.checked,
                self.signatures,
                self.budget,
            )?;
            let function = self
                .original
                .plan
                .callable_body(&self.original.plan.program, self.original.declaration)
                .map_err(|e| source_error(self.budget, &e))?;
            let actual = self
                .checked
                .locals
                .get(local.0)
                .ok_or_else(|| FactsError::invalid(self.budget, "input parameter missing"))?;
            if actual.mutable != function.param_mut.get(parameter).copied().unwrap_or(false) {
                return Err(FactsError::invalid(
                    self.budget,
                    "input parameter mutability changed",
                ));
            }
            return Ok((local, ty));
        }
        let statement = site.statement.ok_or_else(|| {
            FactsError::unsupported(self.budget, "input evidence requires a root source binding")
        })?;
        let ordinal = u32::try_from(statement)
            .map_err(|_| FactsError::resource(self.budget, "input statement overflow"))?;
        if site.node.path.as_ref() != [1, 0, 2, ordinal] {
            return Err(FactsError::invalid(
                self.budget,
                "input binding occurrence changed",
            ));
        }
        let function = self
            .original
            .plan
            .callable_body(&self.original.plan.program, self.original.declaration)
            .map_err(|e| source_error(self.budget, &e))?;
        let Some(Stmt::Let {
            value: source,
            mutable,
            ..
        }) = function.body.statements.get(statement)
        else {
            return Err(FactsError::unsupported(
                self.budget,
                "input evidence requires initialized source let",
            ));
        };
        let local = mapped_source_binding(self.original, index, self.budget)?;
        let position = self
            .original
            .root_positions
            .get(index)
            .copied()
            .flatten()
            .ok_or_else(|| FactsError::invalid(self.budget, "input let mapping missing"))?;
        let Some(ir::TypedStmt::Let {
            local: actual,
            value,
        }) = self.checked.body.statements.get(position)
        else {
            return Err(FactsError::invalid(
                self.budget,
                "input checked let changed",
            ));
        };
        let declaration = self
            .checked
            .locals
            .get(local.0)
            .ok_or_else(|| FactsError::invalid(self.budget, "input checked local missing"))?;
        if *actual != local || declaration.mutable != *mutable {
            return Err(FactsError::invalid(
                self.budget,
                "input binding identity or mutability changed",
            ));
        }
        let annotation = site
            .annotation
            .as_ref()
            .map(|ty| {
                checked_local_ty(ty, self.signatures).map_err(|e| source_error(self.budget, &e))
            })
            .transpose()?;
        let ty = self.expression(
            source,
            &site.node.child(3, 0, self.budget)?,
            *value,
            statement,
            annotation.as_ref(),
            depth + 1,
        )?;
        let annotation_matches = match annotation.as_ref() {
            Some(annotation) => self.equal(annotation, &ty)?,
            None => true,
        };
        if !self.equal(&ty, &declaration.ty)? || !annotation_matches {
            return Err(FactsError::invalid(
                self.budget,
                "input initializer type changed",
            ));
        }
        Ok((local, ty))
    }
    fn operand(
        &self,
        source: &Expr,
        node: &sr::SourceNode,
        before: usize,
        depth: usize,
    ) -> Result<(ir::LocalId, ir::Ty), FactsError> {
        let (occurrence, binding) = self
            .inventory
            .local_operands
            .get(&(source as *const Expr as usize))
            .ok_or_else(|| {
                FactsError::invalid(self.budget, "input lacks lexical operand authority")
            })?;
        if !source_node_equal(occurrence, node, self.budget)? {
            return Err(FactsError::invalid(
                self.budget,
                "input lexical occurrence changed",
            ));
        }
        validate_original_occurrence(self.original, occurrence, self.budget)?;
        validate_original_occurrence(self.original, binding, self.budget)?;
        let index = source_binding_index(&self.original.bindings, binding, self.budget)?
            .ok_or_else(|| FactsError::invalid(self.budget, "input lexical binding missing"))?;
        let site = &self.original.bindings[index];
        if site.parameter.is_none() && !site.statement.is_some_and(|s| s < before) {
            return Err(FactsError::invalid(
                self.budget,
                "input initializer refers to a later binding",
            ));
        }
        self.binding(index, depth + 1)
    }
    fn reference(&self, kind: ir::BorrowKind, referent: ir::Ty) -> Result<ir::Ty, FactsError> {
        sr::retain(self.budget, std::mem::size_of::<ir::Ty>() + 32)?;
        Ok(ir::Ty::Ref {
            kind,
            referent: Arc::new(referent),
        })
    }
    fn expression(
        &self,
        source: &Expr,
        node: &sr::SourceNode,
        id: ir::ExprId,
        before: usize,
        expected: Option<&ir::Ty>,
        depth: usize,
    ) -> Result<ir::Ty, FactsError> {
        sr::depth(self.budget, depth)?;
        sr::work(self.budget, 1)?;
        let actual = self
            .checked
            .exprs
            .get(id.0)
            .ok_or_else(|| FactsError::invalid(self.budget, "input expression missing"))?;
        let ty = match (source, &actual.kind) {
            (Expr::Var(path), ir::ExprKind::Local(local)) if is_local_shaped_path(path) => {
                let (root, ty) = self.operand(source, node, before, depth)?;
                if root != *local {
                    return Err(FactsError::invalid(self.budget, "input alias root changed"));
                }
                ty
            }
            (
                Expr::Borrow { kind, place },
                ir::ExprKind::Borrow {
                    kind: actual_kind,
                    place: actual_place,
                    timing: ir::BorrowTiming::Immediate,
                },
            ) => {
                let (root, mut ty) = self.operand(source, node, before, depth)?;
                let checker = Checker::new(
                    self.signatures,
                    ir::Ty::Unit,
                    self.original
                        .plan
                        .callable_context(self.original.declaration)
                        .map_err(|e| source_error(self.budget, &e))?
                        .module,
                );
                let mut projections = Vec::new();
                for selector in &place.fields {
                    sr::work(self.budget, 1)?;
                    let (projection, next) = match selector {
                        FieldSelector::Deref => {
                            let ir::Ty::Ref { referent, .. } = &ty else {
                                return Err(FactsError::unsupported(
                                    self.budget,
                                    "input dereference requires a reference",
                                ));
                            };
                            (ir::Projection::Deref, referent.as_ref().clone())
                        }
                        _ => {
                            let (index, next) = checker
                                .field_type(&ty, selector)
                                .map_err(|e| source_error(self.budget, &e))?;
                            (ir::Projection::Field(index), next)
                        }
                    };
                    sr::push(&mut projections, projection, self.budget)?;
                    ty = next;
                }
                if root != actual_place.local
                    || projections != actual_place.projection
                    || kind != actual_kind
                {
                    return Err(FactsError::invalid(
                        self.budget,
                        "input borrow place or kind changed",
                    ));
                }
                self.reference(*kind, ty)?
            }
            (Expr::Integer(literal), ir::ExprKind::Int(value)) => {
                let resolved = resolve_integer(literal, false, expected)
                    .map_err(|e| source_error(self.budget, &e))?;
                if resolved != *value {
                    return Err(FactsError::invalid(self.budget, "input integer changed"));
                }
                ir::Ty::Int(resolved.ty)
            }
            (Expr::Bool(a), ir::ExprKind::Bool(b)) if a == b => ir::Ty::Bool,
            (Expr::Char(a), ir::ExprKind::Char(b)) if u32::from(*a) == b.value => ir::Ty::Char,
            (Expr::Tuple(fields), ir::ExprKind::Tuple(values))
            | (Expr::Array(fields), ir::ExprKind::Array(values))
                if fields.len() == values.len() =>
            {
                let mut types = Vec::new();
                for (index, (field, value)) in fields.iter().zip(values).enumerate() {
                    let hint = match expected {
                        Some(ir::Ty::Tuple(types)) => types.get(index),
                        Some(ir::Ty::Array { element, .. }) => Some(element.as_ref()),
                        _ => None,
                    };
                    sr::push(
                        &mut types,
                        self.expression(
                            field,
                            &node.child(12, index, self.budget)?,
                            *value,
                            before,
                            hint,
                            depth + 1,
                        )?,
                        self.budget,
                    )?;
                }
                if matches!(source, Expr::Tuple(_)) {
                    ir::Ty::Tuple(types.into())
                } else {
                    let element = types
                        .first()
                        .cloned()
                        .or_else(|| match expected {
                            Some(ir::Ty::Array { element, .. }) => Some(element.as_ref().clone()),
                            _ => None,
                        })
                        .ok_or_else(|| {
                            FactsError::unsupported(
                                self.budget,
                                "empty array requires source annotation",
                            )
                        })?;
                    for ty in &types {
                        if !self.equal(&element, ty)? {
                            return Err(FactsError::invalid(
                                self.budget,
                                "input array elements differ",
                            ));
                        }
                    }
                    sr::retain(self.budget, std::mem::size_of::<ir::Ty>() + 32)?;
                    ir::Ty::Array {
                        element: Arc::new(element),
                        len: fields.len(),
                    }
                }
            }
            (
                Expr::Repeat { value, len },
                ir::ExprKind::Repeat {
                    value: actual_value,
                    len: actual_len,
                },
            ) if len == actual_len => {
                let hint = match expected {
                    Some(ir::Ty::Array { element, .. }) => Some(element.as_ref()),
                    _ => None,
                };
                let element = self.expression(
                    value,
                    &node.child(11, 0, self.budget)?,
                    *actual_value,
                    before,
                    hint,
                    depth + 1,
                )?;
                sr::retain(self.budget, std::mem::size_of::<ir::Ty>() + 32)?;
                ir::Ty::Array {
                    element: Arc::new(element),
                    len: *len,
                }
            }
            (Expr::Struct { name, fields }, ir::ExprKind::Struct { fields: values, .. }) => {
                source_clone::clone_vector(fields, self.budget)
                    .map_err(|e| source_error(self.budget, &e))?;
                self.nominal(
                    name,
                    None,
                    fields
                        .iter()
                        .map(|(name, value)| (FieldSelector::Named(*name), value))
                        .collect(),
                    values,
                    node,
                    before,
                    expected,
                    depth,
                )?
            }
            (Expr::TupleStruct { name, fields }, ir::ExprKind::Struct { fields: values, .. }) => {
                source_clone::clone_vector(fields, self.budget)
                    .map_err(|e| source_error(self.budget, &e))?;
                self.nominal(
                    name,
                    None,
                    fields
                        .iter()
                        .enumerate()
                        .map(|(index, value)| (FieldSelector::Tuple(index), value))
                        .collect(),
                    values,
                    node,
                    before,
                    expected,
                    depth,
                )?
            }
            (
                Expr::EnumVariant {
                    name,
                    shape,
                    fields,
                },
                ir::ExprKind::EnumVariant {
                    variant,
                    fields: values,
                    ..
                },
            ) => {
                let owner = name
                    .semantic_parent(&*self.original.plan.program.names)
                    .map_err(native_adapter::name_diagnostic)
                    .map_err(|e| source_error(self.budget, &e))?;
                let member = *name.segments().last().ok_or_else(|| {
                    FactsError::invalid(self.budget, "input enum variant name missing")
                })?;
                source_clone::clone_vector(fields, self.budget)
                    .map_err(|e| source_error(self.budget, &e))?;
                self.nominal(
                    &owner,
                    Some((member, *shape, *variant)),
                    fields
                        .iter()
                        .map(|(selector, value)| (selector.clone(), value))
                        .collect(),
                    values,
                    node,
                    before,
                    expected,
                    depth,
                )?
            }
            _ => {
                return Err(FactsError::unsupported(
                    self.budget,
                    "input initializer has no recursive source evidence",
                ));
            }
        };
        if let ir::ExprKind::Struct { ty: tag, .. } = &actual.kind {
            if !self.equal(&ty, &ir::Ty::Struct(tag.clone()))? {
                return Err(FactsError::invalid(
                    self.budget,
                    "input nominal constructor tag changed",
                ));
            }
        }
        if let ir::ExprKind::EnumVariant { ty: tag, .. } = &actual.kind {
            if !self.equal(&ty, &ir::Ty::Enum(tag.clone()))? {
                return Err(FactsError::invalid(
                    self.budget,
                    "input enum constructor tag changed",
                ));
            }
        }
        if actual.completion != ir::Completion::Returns || !self.equal(&ty, &actual.ty)? {
            return Err(FactsError::invalid(
                self.budget,
                "input source expression type or completion changed",
            ));
        }
        Ok(ty)
    }
    #[allow(clippy::too_many_arguments)]
    fn nominal(
        &self,
        name: &AstPath,
        variant: Option<(AstIdent, ir::VariantShape, usize)>,
        fields: Vec<(FieldSelector, &Expr)>,
        values: &[(usize, ir::ExprId)],
        node: &sr::SourceNode,
        before: usize,
        expected: Option<&ir::Ty>,
        depth: usize,
    ) -> Result<ir::Ty, FactsError> {
        sr::retain(
            self.budget,
            fields
                .len()
                .saturating_mul(std::mem::size_of::<(FieldSelector, &Expr)>()),
        )?;
        if fields.len() != values.len() {
            return Err(FactsError::invalid(
                self.budget,
                "input nominal field count changed",
            ));
        }
        let checker = Checker::new(
            self.signatures,
            ir::Ty::Unit,
            self.original
                .plan
                .callable_context(self.original.declaration)
                .map_err(|e| source_error(self.budget, &e))?
                .module,
        );
        let key = checker
            .path_key(name, Namespace::Type)
            .map_err(|e| source_error(self.budget, &e))?;
        let decl = *self
            .original
            .plan
            .registry
            .nominals
            .get(&key)
            .ok_or_else(|| {
                FactsError::invalid(self.budget, "input nominal source declaration missing")
            })?;
        let (env, schema) = self
            .original
            .plan
            .nominal_schema(decl, self.budget)
            .map_err(|e| source_error(self.budget, &e))?;
        let selected_variant = match variant {
            Some((member, shape, actual_index)) => {
                let index = checker
                    .variant_index(decl, member)
                    .map_err(|e| source_error(self.budget, &e))?;
                let ir::Ty::Enum(enumeration) = &schema else {
                    return Err(FactsError::invalid(
                        self.budget,
                        "input variant owner is not enum",
                    ));
                };
                let definition = enumeration.def().variants.get(index).ok_or_else(|| {
                    FactsError::invalid(self.budget, "input variant definition missing")
                })?;
                if index != actual_index
                    || (shape != definition.shape
                        && !(definition.shape == ir::VariantShape::Unit
                            && shape == ir::VariantShape::Struct
                            && fields.is_empty()))
                {
                    return Err(FactsError::invalid(
                        self.budget,
                        "input enum variant or shape changed",
                    ));
                }
                if definition.fields.len() != fields.len() {
                    return Err(FactsError::invalid(
                        self.budget,
                        "input enum field arity changed",
                    ));
                }
                Some(index)
            }
            None => None,
        };
        let mut state = sr::CallInferenceState::new(Some(env), self.budget)?;
        if let Some(expected) = expected {
            state.constrain_input(&schema, expected)?;
        }
        let mut source_types = Vec::new();
        for (position, ((selector, source), (actual_index, value))) in
            fields.iter().zip(values).enumerate()
        {
            let (index, formal) =
                self.nominal_field(&checker, &schema, selected_variant, selector)?;
            if index != *actual_index {
                return Err(FactsError::invalid(
                    self.budget,
                    "input nominal field identity changed",
                ));
            }
            let hint = expected
                .map(|ty| {
                    self.nominal_field(&checker, ty, selected_variant, selector)
                        .map(|(_, ty)| ty)
                })
                .transpose()?;
            let input = self.expression(
                source,
                &node.child(14, position, self.budget)?,
                *value,
                before,
                hint.as_ref(),
                depth + 1,
            )?;
            state.constrain_input(&formal, &input)?;
            sr::push(&mut source_types, (selector.clone(), input), self.budget)?;
        }
        let mut materialize = |decl, args: &[ir::Ty]| {
            self.original
                .plan
                .intern_nominal(
                    decl,
                    args,
                    ir::FactsCtx::Validated(&self.original.facts),
                    self.budget,
                )
                .map_err(|e| source_error(self.budget, &e))
        };
        let args = state.finish(false, &mut materialize)?.ok_or_else(|| {
            FactsError::unsupported(self.budget, "input nominal actuals remain unresolved")
        })?;
        let ty = materialize(decl, &args)?;
        for (selector, input) in source_types {
            let (_, field) = self.nominal_field(&checker, &ty, selected_variant, &selector)?;
            if !self.equal(&field, &input)? {
                return Err(FactsError::invalid(
                    self.budget,
                    "input nominal field type changed",
                ));
            }
        }
        Ok(ty)
    }
    fn nominal_field(
        &self,
        checker: &Checker<'_>,
        ty: &ir::Ty,
        variant: Option<usize>,
        selector: &FieldSelector,
    ) -> Result<(usize, ir::Ty), FactsError> {
        let Some(variant) = variant else {
            return checker
                .field_type(ty, selector)
                .map_err(|e| source_error(self.budget, &e));
        };
        let ir::Ty::Enum(enumeration) = ty else {
            return Err(FactsError::invalid(
                self.budget,
                "input enum field owner changed",
            ));
        };
        let definition =
            enumeration.def().variants.get(variant).ok_or_else(|| {
                FactsError::invalid(self.budget, "input enum field variant missing")
            })?;
        let index = match selector {
            FieldSelector::Tuple(index) if definition.shape == ir::VariantShape::Tuple => *index,
            FieldSelector::Named(name) if definition.shape == ir::VariantShape::Struct => checker
                .member_index(enumeration.def().source_decl, Some(variant), *name)
                .map_err(|e| source_error(self.budget, &e))?,
            _ => {
                return Err(FactsError::invalid(
                    self.budget,
                    "input enum field selector changed",
                ));
            }
        };
        let fields = definition
            .fields
            .read(self.budget)
            .map_err(|e| source_error(self.budget, &e))?;
        let field = fields
            .get(index)
            .ok_or_else(|| FactsError::invalid(self.budget, "input enum field missing"))?;
        Ok((index, field.ty.clone()))
    }
}
