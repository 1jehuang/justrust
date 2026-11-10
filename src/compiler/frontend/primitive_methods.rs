//! Exact finite syntax authority for loader-owned primitive inherent methods.
use super::*;

#[derive(Clone, Debug)]
enum Event {
    Tag(&'static str, usize),
    Name(AstIdent),
    Path(AstPath),
    Visibility(ir::Visibility),
    Anchor(PathAnchor),
    Int(ir::IntTy),
    Integer(u128, Option<ir::IntTy>),
}
type Emit<'a> = dyn FnMut(Event) -> Result<(), String> + 'a;
fn exact_path(names: &NameStore, a: &AstPath, b: &AstPath) -> Result<bool, String> {
    let mut equal = true;
    for (a, b) in [
        (a.clone(), b.clone()),
        (
            a.original_use(names)
                .map_err(native_adapter::name_diagnostic)?,
            b.original_use(names)
                .map_err(native_adapter::name_diagnostic)?,
        ),
    ] {
        a.check(names).map_err(native_adapter::name_diagnostic)?;
        b.check(names).map_err(native_adapter::name_diagnostic)?;
        names
            .budget()
            .charge(
                crate::compiler::expansion_budget::Counter::Work,
                a.len()
                    .checked_add(b.len())
                    .ok_or("primitive method path work overflow")? as u64,
            )
            .map_err(|e| e.to_string())?;
        equal &= a.anchor() == b.anchor()
            && a.len() == b.len()
            && a.catalog_callable() == b.catalog_callable();
        for (a, b) in a.segments().iter().zip(b.segments()) {
            equal &= exact_source_ident(names, *a, *b)?;
        }
    }
    Ok(equal)
}
fn tag(emit: &mut Emit<'_>, name: &'static str, value: usize) -> Result<(), String> {
    emit(Event::Tag(name, value))
}
fn region(value: &Option<RegionName>, emit: &mut Emit<'_>) -> Result<(), String> {
    match value {
        None => tag(emit, "no-region", 0),
        Some(RegionName::Static) => tag(emit, "static", 0),
        Some(RegionName::Infer) => tag(emit, "infer-region", 0),
        Some(RegionName::Named(name)) => {
            tag(emit, "region", 0)?;
            emit(Event::Name(*name))
        }
    }
}
fn regions(value: &RegionDecls, emit: &mut Emit<'_>) -> Result<(), String> {
    region(&value.receiver, emit)?;
    tag(emit, "regions", value.params.len())?;
    for name in &value.params {
        emit(Event::Name(*name))?;
    }
    tag(emit, "outlives", value.outlives.len())?;
    for (a, b) in &value.outlives {
        region(&Some(*a), emit)?;
        region(&Some(*b), emit)?;
    }
    Ok(())
}
fn ty(value: &Type, depth: usize, emit: &mut Emit<'_>) -> Result<(), String> {
    if depth > 64 {
        return Err("unsupported: primitive method source type depth".into());
    }
    match value {
        Type::Bool => tag(emit, "bool", 0)?,
        Type::SelfType => tag(emit, "Self", 0)?,
        Type::Unit => tag(emit, "unit", 0)?,
        Type::Infer => tag(emit, "infer", 0)?,
        Type::Int(value) => emit(Event::Int(*value))?,
        Type::Param(name) => {
            tag(emit, "parameter", 0)?;
            emit(Event::Name(*name))?;
        }
        Type::Named(path) => {
            tag(emit, "named", 0)?;
            emit(Event::Path(path.clone()))?;
        }
        Type::Apply(path) => {
            tag(emit, "apply", path.segments.len())?;
            emit(Event::Anchor(path.anchor.clone()))?;
            tag(emit, "resolved-use", path.resolved_use.is_some() as usize)?;
            if let Some(path) = &path.resolved_use {
                emit(Event::Path(path.clone()))?;
            }
            for segment in &path.segments {
                emit(Event::Name(segment.name))?;
                tag(emit, "arguments", segment.args.len())?;
                for value in &segment.args {
                    ty(value, depth + 1, emit)?;
                }
                tag(emit, "region-arguments", segment.region_args.len())?;
                for value in &segment.region_args {
                    region(&Some(*value), emit)?;
                }
                tag(emit, "associated-types", segment.associated_types.len())?;
                for binding in &segment.associated_types {
                    emit(Event::Name(binding.name))?;
                    ty(&binding.value, depth + 1, emit)?;
                }
            }
        }
        Type::Tuple(values) => {
            tag(emit, "tuple-type", values.len())?;
            for value in values {
                ty(value, depth + 1, emit)?;
            }
        }
        Type::Ref {
            kind,
            region: lifetime,
            referent,
        } => {
            tag(emit, "reference", *kind as usize)?;
            region(lifetime, emit)?;
            ty(referent, depth + 1, emit)?;
        }
        _ => return Err("unsupported: primitive inherent method source type syntax".into()),
    }
    Ok(())
}
fn expression(value: &Expr, depth: usize, emit: &mut Emit<'_>) -> Result<(), String> {
    if depth > 64 {
        return Err("unsupported: primitive method source expression depth".into());
    }
    match value {
        Expr::Var(path) => {
            tag(emit, "variable", 0)?;
            emit(Event::Path(path.clone()))?;
        }
        Expr::Call { name, args } => {
            tag(emit, "call", args.len())?;
            emit(Event::Path(name.clone()))?;
            for value in args {
                expression(value, depth + 1, emit)?;
            }
        }
        Expr::If {
            condition,
            then_block,
            else_block,
        } => {
            tag(emit, "if", 0)?;
            expression(condition, depth + 1, emit)?;
            body(then_block, depth + 1, emit)?;
            body(else_block, depth + 1, emit)?;
        }
        Expr::EnumVariant {
            name,
            shape,
            fields,
        } => {
            tag(emit, "variant", *shape as usize)?;
            emit(Event::Path(name.clone()))?;
            tag(emit, "fields", fields.len())?;
            for (field, value) in fields {
                match field {
                    FieldSelector::Named(name) => {
                        tag(emit, "named-field", 0)?;
                        emit(Event::Name(*name))?;
                    }
                    FieldSelector::Tuple(index) => tag(emit, "indexed-field", *index)?,
                    _ => return Err("unsupported: primitive method field selector source".into()),
                }
                expression(value, depth + 1, emit)?;
            }
        }
        Expr::Bool(value) => tag(emit, "boolean", *value as usize)?,
        Expr::Integer(value) => emit(Event::Integer(value.magnitude, value.suffix))?,
        Expr::Tuple(values) => {
            tag(emit, "tuple", values.len())?;
            for value in values {
                expression(value, depth + 1, emit)?;
            }
        }
        _ => return Err("unsupported: primitive inherent method source expression syntax".into()),
    }
    Ok(())
}
fn binding(value: BindingName, emit: &mut Emit<'_>) -> Result<(), String> {
    match value {
        BindingName::Named(name) => {
            tag(emit, "binding", 0)?;
            emit(Event::Name(name))
        }
        BindingName::Wildcard => tag(emit, "wildcard", 0),
    }
}
fn body(value: &Block, depth: usize, emit: &mut Emit<'_>) -> Result<(), String> {
    if depth > 64 {
        return Err("unsupported: primitive method source block depth".into());
    }
    tag(emit, "block", value.statements.len())?;
    for statement in &value.statements {
        match statement {
            Stmt::Let {
                name,
                mutable,
                ty: annotation,
                value,
            } => {
                tag(emit, "let", *mutable as usize)?;
                binding(*name, emit)?;
                ty(annotation, depth + 1, emit)?;
                expression(value, depth + 1, emit)?;
            }
            Stmt::Return(value) => {
                tag(emit, "return", value.is_some() as usize)?;
                if let Some(value) = value {
                    expression(value, depth + 1, emit)?;
                }
            }
            Stmt::Expr(value) | Stmt::ImplicitExpr(value) => {
                tag(
                    emit,
                    "statement",
                    matches!(statement, Stmt::ImplicitExpr(_)) as usize,
                )?;
                expression(value, depth + 1, emit)?;
            }
            _ => {
                return Err("unsupported: primitive inherent method source statement syntax".into());
            }
        }
    }
    tag(emit, "tail", value.tail.is_some() as usize)?;
    if let Some(value) = &value.tail {
        expression(value, depth + 1, emit)?;
    }
    Ok(())
}
fn recipe(block: &ImplDecl, emit: &mut Emit<'_>) -> Result<(), String> {
    if block.trait_path.is_some()
        || !block.const_params.is_empty()
        || !block.type_params.is_empty()
        || !block.bounds.is_empty()
        || !block.associated_types.is_empty()
        || !block.associated_constants.is_empty()
        || block.unsafety
        || block.items.is_empty()
    {
        return Err("unsupported: primitive inherent method impl source shape".into());
    }
    ty(&block.self_ty, 0, emit)?;
    regions(&block.regions, emit)?;
    tag(emit, "methods", block.items.len())?;
    for item in &block.items {
        let function = &item.function;
        if function.module != block.module
            || function.lexical_scope.is_some()
            || function.static_mut
            || !function.bounds.is_empty()
            || !function.const_params.is_empty()
            || function.impl_params != 0
            || function.target_feature.is_some()
            || function.embedded_source.is_some()
            || function.suspended_source.is_some()
            || function.direct_return_source.is_some()
        {
            return Err("unsupported: primitive inherent method source header".into());
        }
        emit(Event::Visibility(item.visibility.clone()))?;
        emit(Event::Visibility(function.visibility.clone()))?;
        tag(
            emit,
            "receiver",
            match item.receiver {
                ReceiverKind::None => 0,
                ReceiverKind::Value { mutable: false } => 1,
                ReceiverKind::Value { mutable: true } => 2,
                ReceiverKind::Borrowed {
                    kind: ir::BorrowKind::Shared,
                } => 3,
                ReceiverKind::Borrowed {
                    kind: ir::BorrowKind::Mut,
                } => 4,
            },
        )?;
        tag(emit, "constness", function.constness as usize)?;
        tag(emit, "unsafe", function.unsafety as usize)?;
        emit(Event::Path(function.name.clone()))?;
        regions(&function.regions, emit)?;
        tag(emit, "generics", function.type_params.len())?;
        for name in &function.type_params {
            emit(Event::Name(*name))?;
        }
        tag(emit, "parameters", function.params.len())?;
        tag(emit, "parameter-mutability", function.param_mut.len())?;
        for mutable in &function.param_mut {
            tag(emit, "mutable", *mutable as usize)?;
        }
        for (name, annotation) in &function.params {
            binding(*name, emit)?;
            ty(annotation, 0, emit)?;
        }
        ty(&function.result, 0, emit)?;
        body(&function.body, 0, emit)?;
    }
    Ok(())
}
#[derive(Clone, Debug)]
pub(super) struct Methods {
    events: Arc<[Event]>,
    resolved: bool,
    _admitted: crate::compiler::expansion_budget::RetainedBytes,
}
impl Methods {
    pub(super) fn capture(
        block: &ImplDecl,
        budget: &crate::compiler::expansion_budget::ExpansionBudget,
    ) -> Result<Self, String> {
        let mut count = 0usize;
        let mut path_bytes = 0usize;
        recipe(block, &mut |event| {
            budget
                .charge(crate::compiler::expansion_budget::Counter::Work, 1)
                .map_err(|e| e.to_string())?;
            count = count
                .checked_add(1)
                .ok_or("primitive method event overflow")?;
            if let Event::Path(path) = event {
                path_bytes = path_bytes
                    .checked_add(
                        path.len()
                            .checked_mul(std::mem::size_of::<AstIdent>())
                            .ok_or("primitive method path overflow")?,
                    )
                    .ok_or("primitive method path overflow")?;
            }
            Ok(())
        })?;
        let bytes = count
            .checked_mul(std::mem::size_of::<Event>())
            .and_then(|n| n.checked_add(path_bytes))
            .and_then(|n| n.checked_add(std::mem::size_of::<Self>()))
            .ok_or("primitive method source allocation overflow")?;
        let admitted = budget
            .reserve(
                crate::compiler::expansion_budget::Storage::Retained,
                bytes as u64,
            )
            .and_then(|lease| lease.retain())
            .map_err(|e| e.to_string())?;
        let mut events = Vec::new();
        events
            .try_reserve_exact(count)
            .map_err(|_| "primitive method source allocation failed")?;
        recipe(block, &mut |event| {
            budget
                .charge(crate::compiler::expansion_budget::Counter::Work, 1)
                .map_err(|e| e.to_string())?;
            events.push(event);
            Ok(())
        })?;
        Ok(Self {
            events: events.into(),
            resolved: false,
            _admitted: admitted,
        })
    }
    pub(super) fn replay(&self, block: &ImplDecl, names: &NameStore) -> Result<(), String> {
        let mut index = 0usize;
        recipe(block, &mut |event| {
            names
                .budget()
                .charge(crate::compiler::expansion_budget::Counter::Work, 1)
                .map_err(|e| e.to_string())?;
            let equal = match (self.events.get(index), &event) {
                (Some(Event::Tag(a, b)), Event::Tag(c, d)) => a == c && b == d,
                (Some(Event::Name(a)), Event::Name(b)) => exact_source_ident(names, *a, *b)?,
                (Some(Event::Path(a)), Event::Path(b)) => exact_path(names, a, b)?,
                (Some(Event::Anchor(a)), Event::Anchor(b)) => a == b,
                (Some(Event::Visibility(a)), Event::Visibility(b)) => a == b,
                (Some(Event::Int(a)), Event::Int(b)) => a == b,
                (Some(Event::Integer(a, b)), Event::Integer(c, d)) => a == c && b == d,
                _ => false,
            };
            if !equal {
                return Err(
                    "primitive inherent method changed exact source member, signature or body"
                        .into(),
                );
            }
            index += 1;
            Ok(())
        })?;
        if index != self.events.len() {
            return Err("primitive inherent method changed complete source syntax".into());
        }
        Ok(())
    }
    pub(super) fn require_resolved(&self) -> Result<(), String> {
        if self.resolved {
            Ok(())
        } else {
            Err("primitive method requires owning source resolution".into())
        }
    }
    pub(super) fn charge_registration(&self, budget: &ir::PlanBudget) -> Result<(), String> {
        budget.charge_work(self.events.len())?;
        for event in self.events.iter() {
            if let Event::Path(path) = event {
                budget.charge_work(path.len())?;
            }
        }
        Ok(())
    }
    pub(super) fn require_original(&self) -> Result<(), String> {
        if self.resolved {
            return Err("primitive method source resolved twice".into());
        }
        Ok(())
    }
    pub(super) fn into_resolved(mut self) -> Result<Self, String> {
        self.require_original()?;
        self.resolved = true;
        Ok(self)
    }
}
