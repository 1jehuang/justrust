//! Source-occurrence authority for the bounded canonical scalar Cell operations.
use super::*;

#[derive(Clone, Debug)]
enum Origin {
    Associated {
        path: Arc<GenericPath>,
        occurrence: AstPath,
    },
    Method {
        name: AstIdent,
        pointer: usize,
    },
}
#[derive(Clone, Debug)]
pub struct CellSourceOperation {
    origin: Origin,
    owner: ir::DeclId,
    declaration: ir::DeclId,
    operation: CellOperation,
    site: ir::ExprId,
    receiver: Option<ir::ExprId>,
    value: Option<ir::ExprId>,
}
// Equality is bounded immutable occurrence identity, not recursive source/type equality.
impl PartialEq for CellSourceOperation {
    fn eq(&self, other: &Self) -> bool {
        let origin = match (&self.origin, &other.origin) {
            (Origin::Associated { path: a, .. }, Origin::Associated { path: b, .. }) => {
                Arc::ptr_eq(a, b)
            }
            (Origin::Method { pointer: a, .. }, Origin::Method { pointer: b, .. }) => a == b,
            _ => false,
        };
        origin
            && self.owner == other.owner
            && self.declaration == other.declaration
            && self.operation == other.operation
            && self.site == other.site
            && self.receiver == other.receiver
            && self.value == other.value
    }
}
impl Eq for CellSourceOperation {}
fn source_body<'a>(
    plan: &'a PlanCtx,
    owner: ir::DeclId,
    budget: &ir::PlanBudget,
) -> Result<&'a Block, String> {
    plan.check_budget(budget)?;
    let function = plan
        .registry
        .source_callable(owner)
        .ok_or("invalid metadata: Cell operation has no original source owner")?;
    let key = name_scope::path_key(plan.program.names.as_ref(), &function.name, Namespace::Type)?;
    if let Some(source) = plan.program.closure_sources.get(&key)
        && let Some(stored) = &source.stored
    {
        let parent = plan.registry.closure_source_owner(source)?;
        if plan.stored_closure_owner(owner)? != Some((parent, stored.site)) {
            return Err("invalid metadata: Cell closure source owner/site changed".into());
        }
        plan.closure_publication_owner(owner, budget)?;
        return Ok(&stored.body);
    }
    Ok(&function.body)
}

impl CellOperation {
    fn catalog(self) -> tr::CatalogItem {
        match self {
            Self::New => tr::CatalogItem::CellNew,
            Self::Get => tr::CatalogItem::CellGet,
            Self::Set => tr::CatalogItem::CellSet,
        }
    }
    fn atom(self) -> super::super::semantic_name::BuiltinAtom {
        use super::super::semantic_name::BuiltinAtom;
        match self {
            Self::New => BuiltinAtom::CellNew,
            Self::Get => BuiltinAtom::CellGet,
            Self::Set => BuiltinAtom::CellSet,
        }
    }
}
pub(super) fn method_operation(
    program: &Program,
    name: AstIdent,
) -> Result<Option<CellOperation>, String> {
    for operation in [CellOperation::Get, CellOperation::Set] {
        if program
            .symbols
            .matches(program.names.as_ref(), name, operation.atom())?
        {
            return Ok(Some(operation));
        }
    }
    Ok(None)
}
impl CellSourceOperation {
    #[cfg(test)]
    pub(in crate::compiler) fn declaration(&self) -> ir::DeclId {
        self.declaration
    }
    pub(in crate::compiler) fn validate(
        &self,
        catalog: &tr::CoreCatalog,
        budget: &ir::PlanBudget,
    ) -> Result<(), String> {
        budget.charge_work(1)?;
        if self.declaration != catalog.id(self.operation.catalog()) {
            return Err("invalid metadata: foreign canonical Cell operation".into());
        }
        let shape = match self.operation {
            CellOperation::New => self.receiver.is_none() && self.value.is_some(),
            CellOperation::Get => self.receiver.is_some() && self.value.is_none(),
            CellOperation::Set => self.receiver.is_some() && self.value.is_some(),
        };
        if !shape {
            return Err("invalid metadata: Cell operation operand roles changed".into());
        }
        Ok(())
    }
    pub(in crate::compiler) fn validate_site(
        &self,
        owner: ir::DeclId,
        declaration: ir::DeclId,
        site: ir::ExprId,
        receiver: Option<ir::ExprId>,
        value: Option<ir::ExprId>,
    ) -> Result<(), String> {
        if self.owner != owner
            || self.declaration != declaration
            || self.site != site
            || self.receiver != receiver
            || self.value != value
        {
            return Err("invalid metadata: original Cell operation site changed".into());
        }
        Ok(())
    }
    fn mint(
        plan: &PlanCtx,
        owner: ir::DeclId,
        origin: Origin,
        operation: CellOperation,
        site: ir::ExprId,
        receiver: Option<ir::ExprId>,
        value: Option<ir::ExprId>,
    ) -> Result<Arc<Self>, String> {
        let proof = Self {
            origin,
            owner,
            declaration: plan.catalog.id(operation.catalog()),
            operation,
            site,
            receiver,
            value,
        };
        proof.validate(&plan.catalog, &plan.budget)?;
        proof.replay_original(plan, &plan.budget)?;
        plan.budget
            .retain_bytes(std::mem::size_of::<Self>() + 2 * std::mem::size_of::<usize>())?;
        Ok(Arc::new(proof))
    }
    fn replay_original(&self, plan: &PlanCtx, budget: &ir::PlanBudget) -> Result<(), String> {
        plan.check_budget(budget)?;
        self.validate(&plan.catalog, budget)?;
        let body = source_body(plan, self.owner, budget)?;
        let mut matches = 0usize;
        let mut inspect = |node: &DepthNode<'_>, depth: usize| -> Result<(), String> {
            budget.check_depth(depth)?;
            budget.charge_work(1)?;
            match (&self.origin, node) {
                (Origin::Associated { path, occurrence }, DepthNode::Expr(Expr::Cell(call)))
                    if Arc::ptr_eq(path, &call.source.original) =>
                {
                    call.source.require_kind(SourceCatalogKind::Cell)?;
                    if call.operation != self.operation
                        || !exact_source_path(
                            plan.program.names.as_ref(),
                            occurrence,
                            &call.source.occurrence,
                            budget,
                        )?
                    {
                        return Err(
                            "invalid metadata: original Cell associated operation changed".into(),
                        );
                    }
                    let name = path
                        .segments
                        .last()
                        .ok_or("missing original Cell operation")?
                        .name;
                    if !plan.program.symbols.matches(
                        plan.program.names.as_ref(),
                        name,
                        self.operation.atom(),
                    )? {
                        return Err(
                            "invalid metadata: original Cell operation member changed".into()
                        );
                    }
                    matches += 1;
                }
                (
                    Origin::Method { name, pointer },
                    DepthNode::Expr(
                        expr @ Expr::MethodCall {
                            name: actual,
                            type_args,
                            args,
                            ..
                        },
                    ),
                ) if *pointer == *expr as *const Expr as usize
                    && exact_source_ident(plan.program.names.as_ref(), *name, *actual)? =>
                {
                    if method_operation(&plan.program, *actual)? != Some(self.operation)
                        || !type_args.is_empty()
                        || args.len() != usize::from(self.operation == CellOperation::Set)
                    {
                        return Err(
                            "invalid metadata: original Cell method operation changed".into()
                        );
                    }
                    matches += 1;
                }
                _ => {}
            }
            Ok(())
        };
        walk_depth_inspecting(
            DepthNode::Block(body),
            None,
            Some(&plan.program.names),
            Some(&mut inspect),
        )?;
        if matches != 1 {
            return Err("invalid metadata: original Cell operation missing or duplicated".into());
        }
        Ok(())
    }
}

// Resolve the exact original member occurrence, not a spelling or a caller
// supplied type name. Closure providers use their own original source function.
fn method_origin(plan: &PlanCtx, owner: ir::DeclId, name: AstIdent) -> Result<Origin, String> {
    let body = source_body(plan, owner, &plan.budget)?;
    let mut found = None;
    let mut inspect = |node: &DepthNode<'_>, depth: usize| -> Result<(), String> {
        plan.budget.check_depth(depth)?;
        plan.budget.charge_work(1)?;
        if let DepthNode::Expr(expr @ Expr::MethodCall { name: actual, .. }) = node
            && exact_source_ident(plan.program.names.as_ref(), *actual, name)?
            && found.replace(*expr as *const Expr as usize).is_some()
        {
            return Err("invalid metadata: duplicated original Cell member occurrence".into());
        }
        Ok(())
    };
    walk_depth_inspecting(
        DepthNode::Block(body),
        None,
        Some(&plan.program.names),
        Some(&mut inspect),
    )?;
    Ok(Origin::Method {
        name,
        pointer: found.ok_or("invalid metadata: foreign Cell method occurrence")?,
    })
}

pub(super) fn replay_operations(
    original: &OriginalSource,
    checked: &ir::CheckedFunction,
    ordinary: &[Option<usize>],
    seen: &mut [bool],
    budget: &ir::PlanBudget,
) -> Result<Vec<bool>, FactsError> {
    let plan = &original.plan;
    let function = plan
        .registry
        .source_callable(original.declaration)
        .ok_or_else(|| FactsError::invalid(budget, "missing original Cell source owner"))?;
    // Reuse the canonical source inventory and its exact expression pointer map.
    // This is ephemeral per-body validation, not a new world table or fact.
    let mut inventory = None;
    budget
        .charge_work(original.calls.len())
        .map_err(|e| source_error(budget, &e))?;
    budget
        .retain_bytes(original.calls.len())
        .map_err(|e| source_error(budget, &e))?;
    let mut sources = Vec::new();
    sources
        .try_reserve_exact(original.calls.len())
        .map_err(|_| {
            FactsError::infrastructure(budget, "Cell source occurrence allocation failed")
        })?;
    sources.resize(original.calls.len(), false);
    let mut associated = 0usize;
    let mut expected = 0usize;
    let mut inspect = |node: &DepthNode<'_>, depth: usize| -> Result<(), String> {
        budget.check_depth(depth)?;
        budget.charge_work(1)?;
        if matches!(node, DepthNode::Expr(Expr::Cell(_))) {
            expected = expected
                .checked_add(1)
                .ok_or("resource: Cell occurrence count overflow")?;
        }
        Ok(())
    };
    walk_depth_inspecting(
        DepthNode::Block(&function.body),
        None,
        Some(&plan.program.names),
        Some(&mut inspect),
    )
    .map_err(|e| source_error(budget, &e))?;
    let mut last_associated = None;
    for (index, expr) in checked.exprs.iter().enumerate() {
        budget
            .charge_work(1)
            .map_err(|e| source_error(budget, &e))?;
        let (proof, receiver, value, operation) = match &expr.kind {
            ir::ExprKind::CellNew { source, value, .. } => {
                (source, None, Some(*value), CellOperation::New)
            }
            ir::ExprKind::CellGet {
                source, receiver, ..
            } => (source, Some(*receiver), None, CellOperation::Get),
            ir::ExprKind::CellSet {
                source,
                receiver,
                value,
                ..
            } => (source, Some(*receiver), Some(*value), CellOperation::Set),
            _ => continue,
        };
        proof
            .validate_site(
                original.declaration,
                plan.catalog.id(operation.catalog()),
                ir::ExprId(index),
                receiver,
                value,
            )
            .map_err(|e| source_error(budget, &e))?;
        proof
            .replay_original(plan, budget)
            .map_err(|e| source_error(budget, &e))?;
        if std::mem::replace(
            seen.get_mut(index).ok_or_else(|| {
                FactsError::invalid(budget, "Cell operation outside typed source")
            })?,
            true,
        ) {
            return Err(FactsError::invalid(
                budget,
                "duplicate Cell typed source site",
            ));
        }
        match &proof.origin {
            Origin::Method { pointer, .. } => {
                if inventory.is_none() {
                    inventory = Some(SourceInventory::build(
                        &plan.program,
                        function,
                        original.identity.clone(),
                        budget,
                    )?);
                }
                let inventory = inventory
                    .as_ref()
                    .ok_or_else(|| FactsError::invalid(budget, "Cell source inventory missing"))?;
                let index = *inventory.call_pointers.get(pointer).ok_or_else(|| {
                    FactsError::invalid(budget, "Cell method lacks original call row")
                })?;
                let site = original.calls.get(index).ok_or_else(|| {
                    FactsError::invalid(budget, "Cell method source index invalid")
                })?;
                let rebuilt = inventory.calls.get(index).ok_or_else(|| {
                    FactsError::invalid(budget, "Cell method rebuilt index invalid")
                })?;
                // OriginalSource intentionally erases ephemeral argument pointers.
                // The rebuilt row is selected by the retained original Expr identity;
                // complete source row equality was already replayed by the caller.
                if site.node != rebuilt.node
                    || site.args_pointer != 0
                    || ordinary.get(index).copied().flatten().is_some()
                    || std::mem::replace(&mut sources[index], true)
                {
                    return Err(FactsError::invalid(
                        budget,
                        "Cell method changed original call correspondence",
                    ));
                }
            }
            Origin::Associated { path, .. } => {
                let pointer = Arc::as_ptr(path) as usize;
                // Each immutable source path may issue exactly one operation.
                // A paid prefix comparison avoids retaining another proof table.
                if last_associated == Some(pointer) {
                    return Err(FactsError::invalid(
                        budget,
                        "duplicate original Cell associated operation",
                    ));
                }
                for previous in checked.exprs.iter().take(index) {
                    budget
                        .charge_work(1)
                        .map_err(|e| source_error(budget, &e))?;
                    let other = match &previous.kind {
                        ir::ExprKind::CellNew { source, .. }
                        | ir::ExprKind::CellGet { source, .. }
                        | ir::ExprKind::CellSet { source, .. } => source,
                        _ => continue,
                    };
                    if let Origin::Associated { path: other, .. } = &other.origin
                        && Arc::ptr_eq(path, other)
                    {
                        return Err(FactsError::invalid(
                            budget,
                            "duplicate original Cell associated operation",
                        ));
                    }
                }
                last_associated = Some(pointer);
                associated = associated.checked_add(1).ok_or_else(|| {
                    FactsError::resource(budget, "Cell associated count overflow")
                })?;
            }
        }
    }
    if associated != expected {
        return Err(FactsError::invalid(
            budget,
            "missing original Cell associated operation",
        ));
    }
    Ok(sources)
}

/// Replay a checked closure only after its original provider/body authority is
/// established by the existing closure publication protocol.
pub(in crate::compiler::frontend) fn replay_checked_body(
    plan: &PlanCtx,
    owner: ir::DeclId,
    checked: &ir::CheckedFunction,
    budget: &ir::PlanBudget,
) -> Result<(), String> {
    let body = source_body(plan, owner, budget)?;
    let mut expected = 0usize;
    let mut inspect = |node: &DepthNode<'_>, depth: usize| -> Result<(), String> {
        budget.check_depth(depth)?;
        budget.charge_work(1)?;
        if matches!(node, DepthNode::Expr(Expr::Cell(_))) {
            expected = expected
                .checked_add(1)
                .ok_or("resource: closure Cell occurrence count overflow")?;
        }
        Ok(())
    };
    walk_depth_inspecting(
        DepthNode::Block(body),
        None,
        Some(&plan.program.names),
        Some(&mut inspect),
    )?;
    let mut count = 0usize;
    for expr in &checked.exprs {
        budget.charge_work(1)?;
        if matches!(
            expr.kind,
            ir::ExprKind::CellNew { .. }
                | ir::ExprKind::CellGet { .. }
                | ir::ExprKind::CellSet { .. }
        ) {
            count = count
                .checked_add(1)
                .ok_or("resource: Cell closure count overflow")?;
        }
    }
    let bytes = count
        .checked_mul(std::mem::size_of::<(bool, usize)>())
        .ok_or("resource: Cell closure replay storage overflow")?;
    let _storage = budget.reserve_bytes(bytes)?;
    let mut seen = Vec::new();
    seen.try_reserve_exact(count)
        .map_err(|_| "infrastructure: Cell closure replay allocation failed")?;
    let mut associated = 0usize;
    for (index, expr) in checked.exprs.iter().enumerate() {
        budget.charge_work(1)?;
        let (proof, receiver, value, operation) = match &expr.kind {
            ir::ExprKind::CellNew { source, value, .. } => {
                (source, None, Some(*value), CellOperation::New)
            }
            ir::ExprKind::CellGet {
                source, receiver, ..
            } => (source, Some(*receiver), None, CellOperation::Get),
            ir::ExprKind::CellSet {
                source,
                receiver,
                value,
                ..
            } => (source, Some(*receiver), Some(*value), CellOperation::Set),
            _ => continue,
        };
        proof.validate_site(
            owner,
            plan.catalog.id(operation.catalog()),
            ir::ExprId(index),
            receiver,
            value,
        )?;
        proof.replay_original(plan, budget)?;
        let key = match &proof.origin {
            Origin::Associated { path, .. } => {
                associated += 1;
                (true, Arc::as_ptr(path) as usize)
            }
            Origin::Method { pointer, .. } => (false, *pointer),
        };
        budget.charge_work(seen.len())?;
        if seen.contains(&key) {
            return Err("invalid metadata: duplicated closure Cell source occurrence".into());
        }
        seen.push(key);
    }
    if associated != expected {
        return Err("invalid metadata: missing original closure Cell operation".into());
    }
    Ok(())
}

impl Checker<'_> {
    pub(super) fn cell_operation(
        &mut self,
        call: &SourceCellCall,
        expected: Option<&ir::Ty>,
    ) -> Result<(ir::ExprKind, Flow), String> {
        call.source.require_kind(SourceCatalogKind::Cell)?;
        let plan = self.signatures.plan()?;
        let explicit = call
            .source
            .argument
            .as_ref()
            .map(|ty| self.source_type_actual(ty))
            .transpose()?;
        let hint = explicit.or_else(|| match (call.operation, expected) {
            (CellOperation::New, Some(ir::Ty::Cell(cell))) => Some(cell.argument().clone()),
            (CellOperation::Get, Some(ty)) => Some(ty.clone()),
            _ => None,
        });
        let receiver_hint = hint
            .as_ref()
            .map(|ty| -> Result<ir::Ty, String> {
                let cell = ir::CellTy::new(&plan.catalog, ty.clone(), &plan.budget)?;
                plan.budget.retain_bytes(
                    std::mem::size_of::<ir::Ty>() + 2 * std::mem::size_of::<usize>(),
                )?;
                Ok(ir::Ty::Ref {
                    kind: ir::BorrowKind::Shared,
                    referent: Arc::new(ir::Ty::Cell(cell)),
                })
            })
            .transpose()?;
        let receiver = call
            .receiver
            .as_ref()
            .map(|source| self.expr_expected(source, receiver_hint.as_ref()))
            .transpose()?;
        let mut completes = true;
        let descriptor = if let Some(receiver) = receiver {
            let flow = self.flow(receiver);
            completes &= flow.completes;
            match flow.ty {
                ir::Ty::Ref {
                    kind: ir::BorrowKind::Shared,
                    referent,
                } => match referent.as_ref() {
                    ir::Ty::Cell(cell) => Some(cell.clone()),
                    _ => {
                        return Err(
                            "Cell operation requires a canonical shared Cell receiver".into()
                        );
                    }
                },
                _ => return Err("Cell operation requires a shared Cell receiver".into()),
            }
        } else {
            None
        };
        let value_hint = descriptor
            .as_ref()
            .map(|cell| cell.argument())
            .or(hint.as_ref());
        let value = call
            .value
            .as_ref()
            .map(|source| self.expr_expected(source, value_hint))
            .transpose()?;
        if let Some(value) = value {
            let flow = self.flow(value);
            if let Some(hint) = value_hint {
                self.require(&flow, hint)?;
            }
            completes &= flow.completes;
        }
        let descriptor = match descriptor {
            Some(cell) => cell,
            None => {
                let value = value.ok_or("missing Cell new value")?;
                let argument = hint.unwrap_or_else(|| self.exprs[value.0].ty.clone());
                ir::CellTy::new(&plan.catalog, argument, &plan.budget)?
            }
        };
        self.facts()
            .validate_ty_metadata_with_budget(&ir::Ty::Cell(descriptor.clone()), &plan.budget)?;
        let owner = self
            .declaration
            .ok_or("unsupported: Cell operation outside original callable")?;
        let proof = CellSourceOperation::mint(
            plan,
            owner,
            Origin::Associated {
                path: call.source.original.clone(),
                occurrence: call.source.occurrence.clone(),
            },
            call.operation,
            ir::ExprId(self.exprs.len()),
            receiver,
            value,
        )?;
        let (kind, ty) = match call.operation {
            CellOperation::New => (
                ir::ExprKind::CellNew {
                    ty: descriptor.clone(),
                    value: value.ok_or("missing Cell new value")?,
                    source: proof,
                },
                ir::Ty::Cell(descriptor),
            ),
            CellOperation::Get => (
                ir::ExprKind::CellGet {
                    ty: descriptor.clone(),
                    receiver: receiver.ok_or("missing Cell get receiver")?,
                    source: proof,
                },
                descriptor.argument().clone(),
            ),
            CellOperation::Set => (
                ir::ExprKind::CellSet {
                    ty: descriptor,
                    receiver: receiver.ok_or("missing Cell set receiver")?,
                    value: value.ok_or("missing Cell set value")?,
                    source: proof,
                },
                ir::Ty::Unit,
            ),
        };
        Ok((kind, Flow { ty, completes }))
    }
    pub(super) fn cell_method(
        &mut self,
        expression: &Expr,
        _expected: Option<&ir::Ty>,
    ) -> Result<Option<ir::ExprId>, String> {
        let Expr::MethodCall {
            receiver,
            name,
            type_args,
            args,
        } = expression
        else {
            return Ok(None);
        };
        let Some(mut ty) = self.method_receiver_hint(receiver)? else {
            return Ok(None);
        };
        let mut references = 0usize;
        while let ir::Ty::Ref { referent, .. } = ty {
            self.signatures.plan()?.budget.check_depth(references)?;
            self.signatures.plan()?.budget.charge_work(1)?;
            references += 1;
            ty = referent.as_ref().clone();
        }
        let ir::Ty::Cell(descriptor) = ty else {
            return Ok(None);
        };
        let plan = self.signatures.plan()?;
        let operation = method_operation(&plan.program, *name)?
            .ok_or("unsupported: canonical Cell method has no executable provider")?;
        if !type_args.is_empty() {
            return Err("E0107: Cell methods take no type arguments".into());
        }
        if args.len() != usize::from(operation == CellOperation::Set) {
            return Err("E0061: Cell method argument count mismatch".into());
        }
        self.facts()
            .validate_ty_metadata_with_budget(&ir::Ty::Cell(descriptor.clone()), &plan.budget)?;
        let receiver = if let Ok(mut place) = static_source_place(receiver) {
            plan.budget.retain_bytes(
                references
                    .checked_mul(std::mem::size_of::<FieldSelector>())
                    .ok_or("resource: Cell receiver projection overflow")?,
            )?;
            place
                .fields
                .try_reserve_exact(references)
                .map_err(|_| "infrastructure: Cell receiver projection allocation failed")?;
            for _ in 0..references {
                place.fields.push(FieldSelector::Deref);
            }
            let (place, actual, _) = self.checked_place(&place)?;
            if !ir::identical_type(&actual, &ir::Ty::Cell(descriptor.clone()), &plan.budget)? {
                return Err("invalid metadata: Cell receiver place type changed".into());
            }
            self.borrow_expression(ir::BorrowKind::Shared, place)?
        } else if references == 0 {
            self.borrow_value(ir::BorrowKind::Shared, receiver, None)?
        } else {
            return Err(
                "unsupported: Cell reference receiver requires canonical local-root provenance"
                    .into(),
            );
        };
        let mut completes = self.flow(receiver).completes;
        let value = if operation == CellOperation::Set {
            let value = self.expr_expected(&args[0], Some(descriptor.argument()))?;
            self.require(&self.flow(value), descriptor.argument())?;
            completes &= self.flow(value).completes;
            Some(value)
        } else {
            None
        };
        let owner = self
            .declaration
            .ok_or("unsupported: Cell method outside original callable")?;
        let proof = CellSourceOperation::mint(
            plan,
            owner,
            method_origin(plan, owner, *name)?,
            operation,
            ir::ExprId(self.exprs.len()),
            Some(receiver),
            value,
        )?;
        let (kind, ty) = match operation {
            CellOperation::Get => (
                ir::ExprKind::CellGet {
                    ty: descriptor.clone(),
                    receiver,
                    source: proof,
                },
                descriptor.argument().clone(),
            ),
            CellOperation::Set => (
                ir::ExprKind::CellSet {
                    ty: descriptor,
                    receiver,
                    value: value.ok_or("missing Cell set value")?,
                    source: proof,
                },
                ir::Ty::Unit,
            ),
            CellOperation::New => return Err("Cell new is not a receiver method".into()),
        };
        self.finish_checked_expression(expression, kind, Flow { ty, completes })
            .map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cell_original_operation_rejects_transplanted_owner_site_and_operand_roles() {
        let text = "fn make()->core::cell::Cell<usize>{core::cell::Cell::new(7usize)}fn main(){}";
        let source = parse(text).unwrap();
        let budget = ir::PlanBudget::default();
        let plan = PlanCtx::from_program(&source, &budget).unwrap();
        let owner = plan.function_decl(0).unwrap();
        let function = plan.registry.source_callable(owner).unwrap();
        let mut origin = None;
        let mut inspect = |node: &DepthNode<'_>, _: usize| -> Result<(), String> {
            if let DepthNode::Expr(Expr::Cell(call)) = node {
                origin = Some(Origin::Associated {
                    path: call.source.original.clone(),
                    occurrence: call.source.occurrence.clone(),
                });
            }
            Ok(())
        };
        walk_depth_inspecting(
            DepthNode::Block(&function.body),
            None,
            Some(&plan.program.names),
            Some(&mut inspect),
        )
        .unwrap();
        let proof = CellSourceOperation::mint(
            &plan,
            owner,
            origin.unwrap(),
            CellOperation::New,
            ir::ExprId(3),
            None,
            Some(ir::ExprId(2)),
        )
        .unwrap();
        proof.validate(&plan.catalog, &budget).unwrap();
        proof.replay_original(&plan, &budget).unwrap();
        proof
            .validate_site(
                owner,
                proof.declaration(),
                ir::ExprId(3),
                None,
                Some(ir::ExprId(2)),
            )
            .unwrap();
        assert!(
            proof
                .validate_site(
                    owner,
                    proof.declaration(),
                    ir::ExprId(4),
                    None,
                    Some(ir::ExprId(2))
                )
                .is_err()
        );
        assert!(
            proof
                .validate_site(
                    owner,
                    proof.declaration(),
                    ir::ExprId(3),
                    Some(ir::ExprId(2)),
                    None
                )
                .is_err()
        );
        assert!(
            proof
                .validate_site(
                    plan.function_decl(1).unwrap(),
                    proof.declaration(),
                    ir::ExprId(3),
                    None,
                    Some(ir::ExprId(2))
                )
                .is_err()
        );
        let foreign_source = parse(text).unwrap();
        let foreign = PlanCtx::from_program(&foreign_source, &budget).unwrap();
        assert!(proof.replay_original(&foreign, &budget).is_err());
        let mut swapped = (*proof).clone();
        swapped.operation = CellOperation::Get;
        assert!(swapped.validate(&plan.catalog, &budget).is_err());
        let mut erased = (*proof).clone();
        erased.value = None;
        assert!(erased.validate(&plan.catalog, &budget).is_err());
    }
}
