//! Sealed original-source authority for canonical owned-wrapper operations.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UnsafeEnclosure {
    Function,
    Block(usize),
}

#[derive(Clone, Debug)]
pub struct ManuallyDropSourceOperation {
    original: Arc<GenericPath>,
    occurrence: AstPath,
    owner: ir::DeclId,
    declaration: ir::DeclId,
    operation: ManuallyDropOperation,
    enclosure: Option<UnsafeEnclosure>,
    site: ir::ExprId,
    argument: ir::ExprId,
}
impl PartialEq for ManuallyDropSourceOperation {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.original, &other.original)
            && self.owner == other.owner
            && self.declaration == other.declaration
            && self.operation == other.operation
            && self.enclosure == other.enclosure
            && self.site == other.site
            && self.argument == other.argument
    }
}
impl Eq for ManuallyDropSourceOperation {}

impl ManuallyDropOperation {
    fn catalog(self) -> tr::CatalogItem {
        match self {
            Self::New => tr::CatalogItem::ManuallyDropNew,
            Self::IntoInner => tr::CatalogItem::ManuallyDropIntoInner,
            Self::Drop => tr::CatalogItem::ManuallyDropDrop,
        }
    }
    fn atom(self) -> super::super::semantic_name::BuiltinAtom {
        use super::super::semantic_name::BuiltinAtom;
        match self {
            Self::New => BuiltinAtom::ManuallyDropNew,
            Self::IntoInner => BuiltinAtom::ManuallyDropIntoInner,
            Self::Drop => BuiltinAtom::ManuallyDropDrop,
        }
    }
}

impl SourceCatalogType {
    pub(super) fn require_kind(&self, expected: SourceCatalogKind) -> Result<(), String> {
        if self.kind != expected {
            return Err("invalid metadata: catalog source kind changed".into());
        }
        Ok(())
    }
}

impl ManuallyDropSourceOperation {
    pub(in crate::compiler) fn validate(
        &self,
        catalog: &tr::CoreCatalog,
        budget: &ir::PlanBudget,
    ) -> Result<(), String> {
        budget.charge_work(1)?;
        if self.declaration != catalog.id(self.operation.catalog()) {
            return Err("invalid metadata: foreign canonical ManuallyDrop operation".into());
        }
        if self.operation == ManuallyDropOperation::Drop && self.enclosure.is_none() {
            return Err("invalid metadata: missing original unsafe ManuallyDrop enclosure".into());
        }
        Ok(())
    }
    pub(in crate::compiler) fn declaration(&self) -> ir::DeclId {
        self.declaration
    }
    pub(in crate::compiler) fn validate_site(
        &self,
        owner: ir::DeclId,
        declaration: ir::DeclId,
        site: ir::ExprId,
        argument: ir::ExprId,
    ) -> Result<(), String> {
        if self.owner != owner
            || self.declaration != declaration
            || self.site != site
            || self.argument != argument
        {
            return Err("invalid metadata: ManuallyDrop original operation site changed".into());
        }
        Ok(())
    }

    fn mint(
        plan: &PlanCtx,
        owner: ir::DeclId,
        call: &SourceManuallyDropCall,
        site: ir::ExprId,
        argument: ir::ExprId,
        budget: &ir::PlanBudget,
    ) -> Result<Arc<Self>, String> {
        call.source.require_kind(SourceCatalogKind::ManuallyDrop)?;
        let declaration = plan.catalog.id(call.operation.catalog());
        let mut proof = Self {
            original: call.source.original.clone(),
            occurrence: call.source.occurrence.clone(),
            owner,
            declaration,
            operation: call.operation,
            enclosure: None,
            site,
            argument,
        };
        proof.enclosure = proof.replay_original(plan, budget, false)?;
        budget.retain_bytes(std::mem::size_of::<Self>() + 2 * std::mem::size_of::<usize>())?;
        Ok(Arc::new(proof))
    }

    fn replay_original(
        &self,
        plan: &PlanCtx,
        budget: &ir::PlanBudget,
        compare_enclosure: bool,
    ) -> Result<Option<UnsafeEnclosure>, String> {
        plan.check_budget(budget)?;
        if self.declaration != plan.catalog.id(self.operation.catalog()) {
            return Err("invalid metadata: foreign canonical ManuallyDrop operation".into());
        }
        self.original
            .checked_path(plan.program.names.as_ref())
            .map_err(native_adapter::name_diagnostic)?;
        self.occurrence
            .check(plan.program.names.as_ref())
            .map_err(native_adapter::name_diagnostic)?;
        let function = plan
            .registry
            .source_callable(self.owner)
            .ok_or("invalid metadata: ManuallyDrop operation has no original source callable")?;
        let mut ordinal = 0usize;
        let mut unsafe_at = [None; 129];
        let mut found = None;
        let mut matches = 0usize;
        let mut inspect = |node: &DepthNode<'_>, depth: usize| -> Result<(), String> {
            budget.check_depth(depth)?;
            budget.charge_work(1)?;
            if depth >= unsafe_at.len() {
                return Err("resource: original unsafe enclosure depth exceeded".into());
            }
            // Every parent is visited before its children. Replacing only
            // this depth invalidates a previous sibling without scanning
            // unrelated descendant slots. Queries read ancestors only.
            unsafe_at[depth] = None;
            let here = ordinal;
            ordinal = ordinal
                .checked_add(1)
                .ok_or("resource: original operation ordinal overflow")?;
            if let DepthNode::Expr(Expr::Unsafe(_)) = node {
                unsafe_at[depth] = Some(here);
            }
            if let DepthNode::Expr(Expr::ManuallyDrop(call)) = node
                && Arc::ptr_eq(&self.original, &call.source.original)
            {
                matches = matches
                    .checked_add(1)
                    .ok_or("resource: original operation count overflow")?;
                call.source.require_kind(SourceCatalogKind::ManuallyDrop)?;
                if call.operation != self.operation
                    || !exact_source_path(
                        plan.program.names.as_ref(),
                        &self.occurrence,
                        &call.source.occurrence,
                        budget,
                    )?
                {
                    return Err("invalid metadata: original ManuallyDrop operation changed".into());
                }
                let member = call
                    .source
                    .original
                    .segments
                    .last()
                    .ok_or("missing original wrapper operation")?;
                if !plan.program.symbols.matches(
                    plan.program.names.as_ref(),
                    member.name,
                    self.operation.atom(),
                )? {
                    return Err("invalid metadata: original ManuallyDrop member changed".into());
                }
                found = unsafe_at[..depth]
                    .iter()
                    .rev()
                    .find_map(|entry| *entry)
                    .map(UnsafeEnclosure::Block)
                    .or(function.unsafety.then_some(UnsafeEnclosure::Function));
            }
            Ok(())
        };
        walk_depth_inspecting(
            DepthNode::Block(&function.body),
            None,
            Some(&plan.program.names),
            Some(&mut inspect),
        )?;
        if matches != 1 {
            return Err(
                "invalid metadata: original ManuallyDrop call missing or duplicated".into(),
            );
        }
        if self.operation == ManuallyDropOperation::Drop && found.is_none() {
            return Err(
                "E0133: ManuallyDrop::drop requires original unsafe source authority".into(),
            );
        }
        if compare_enclosure && self.enclosure != found {
            return Err("invalid metadata: original unsafe enclosure changed".into());
        }
        Ok(found)
    }
}

pub(super) fn replay_operations(
    original: &OriginalSource,
    checked: &ir::CheckedFunction,
    budget: &ir::PlanBudget,
) -> Result<(), FactsError> {
    let plan = &original.plan;
    let function = plan
        .registry
        .source_callable(original.declaration)
        .ok_or_else(|| FactsError::invalid(budget, "missing original wrapper callable"))?;
    let mut expected = 0usize;
    let mut inspect = |node: &DepthNode<'_>, depth: usize| -> Result<(), String> {
        budget.check_depth(depth)?;
        budget.charge_work(1)?;
        if matches!(node, DepthNode::Expr(Expr::ManuallyDrop(_))) {
            expected = expected
                .checked_add(1)
                .ok_or("resource: wrapper operation count overflow")?;
        }
        Ok(())
    };
    walk_depth_inspecting(
        DepthNode::Block(&function.body),
        None,
        Some(&plan.program.names),
        Some(&mut inspect),
    )
    .map_err(|error| source_error(budget, &error))?;
    let bytes = expected
        .checked_mul(4 * std::mem::size_of::<usize>())
        .ok_or_else(|| FactsError::resource(budget, "wrapper replay storage overflow"))?;
    let _storage = budget
        .reserve_bytes(bytes)
        .map_err(|error| source_error(budget, &error))?;
    let mut seen = std::collections::HashSet::new();
    seen.try_reserve(expected)
        .map_err(|_| FactsError::infrastructure(budget, "wrapper replay allocation failed"))?;
    for (index, expr) in checked.exprs.iter().enumerate() {
        budget
            .charge_work(1)
            .map_err(|error| source_error(budget, &error))?;
        let (proof, value, operation) = match &expr.kind {
            ir::ExprKind::ManuallyDrop { source, value, .. } => {
                (source, *value, ManuallyDropOperation::New)
            }
            ir::ExprKind::ManuallyDropIntoInner { source, value, .. } => {
                (source, *value, ManuallyDropOperation::IntoInner)
            }
            ir::ExprKind::ManuallyDropDrop { source, value, .. } => {
                (source, *value, ManuallyDropOperation::Drop)
            }
            _ => continue,
        };
        proof
            .validate_site(
                original.declaration,
                plan.catalog.id(operation.catalog()),
                ir::ExprId(index),
                value,
            )
            .map_err(|error| source_error(budget, &error))?;
        proof
            .replay_original(plan, budget, true)
            .map_err(|error| source_error(budget, &error))?;
        if !seen.insert(Arc::as_ptr(&proof.original) as usize) {
            return Err(FactsError::invalid(
                budget,
                "duplicate original wrapper operation",
            ));
        }
    }
    if seen.len() != expected {
        return Err(FactsError::invalid(
            budget,
            "missing original wrapper operation",
        ));
    }
    Ok(())
}

impl Checker<'_> {
    pub(super) fn manually_drop_operation(
        &mut self,
        call: &SourceManuallyDropCall,
        expected: Option<&ir::Ty>,
    ) -> Result<(ir::ExprKind, Flow), String> {
        call.source.require_kind(SourceCatalogKind::ManuallyDrop)?;
        call.source
            .original
            .checked_path(self.names()?)
            .map_err(native_adapter::name_diagnostic)?;
        if call.operation == ManuallyDropOperation::Drop && self.unsafe_depth == 0 {
            return Err("E0133: ManuallyDrop::drop requires an unsafe block".into());
        }
        let explicit = call
            .source
            .argument
            .as_ref()
            .map(|ty| self.source_type_actual(ty))
            .transpose()?;
        let hint = if let Some(argument) = explicit {
            Some(argument)
        } else {
            match (call.operation, expected) {
                (ManuallyDropOperation::New, Some(ir::Ty::ManuallyDrop(wrapper))) => {
                    Some(wrapper.argument().clone())
                }
                (ManuallyDropOperation::IntoInner, Some(ty)) => Some(ty.clone()),
                _ => None,
            }
        };
        let input_hint = if let Some(argument) = &hint {
            let plan = self.signatures.plan()?;
            let wrapper = ir::ManuallyDropTy::new(&plan.catalog, argument.clone(), &plan.budget)?;
            let ty = ir::Ty::ManuallyDrop(wrapper);
            self.facts()
                .validate_ty_metadata_with_budget(&ty, &plan.budget)?;
            Some(match call.operation {
                ManuallyDropOperation::New => argument.clone(),
                ManuallyDropOperation::IntoInner => ty,
                ManuallyDropOperation::Drop => {
                    plan.budget.retain_bytes(
                        std::mem::size_of::<ir::Ty>() + 2 * std::mem::size_of::<usize>(),
                    )?;
                    ir::Ty::Ref {
                        kind: ir::BorrowKind::Mut,
                        referent: Arc::new(ty),
                    }
                }
            })
        } else {
            None
        };
        let value = self.expr_expected(&call.value, input_hint.as_ref())?;
        let input = self.flow(value);
        if let Some(hint) = &input_hint {
            self.require(&input, hint)?;
        }
        let plan = self.signatures.plan()?;
        let wrapper = match call.operation {
            ManuallyDropOperation::New => {
                ir::ManuallyDropTy::new(&plan.catalog, input.ty.clone(), &plan.budget)?
            }
            ManuallyDropOperation::IntoInner => match &input.ty {
                ir::Ty::ManuallyDrop(wrapper) => wrapper.clone(),
                _ => {
                    return Err(
                        "ManuallyDrop::into_inner requires a canonical owned wrapper".into(),
                    );
                }
            },
            ManuallyDropOperation::Drop => match &input.ty {
                ir::Ty::Ref {
                    kind: ir::BorrowKind::Mut,
                    referent,
                } => match referent.as_ref() {
                    ir::Ty::ManuallyDrop(wrapper) => wrapper.clone(),
                    _ => {
                        return Err(
                            "ManuallyDrop::drop requires an exclusive canonical wrapper borrow"
                                .into(),
                        );
                    }
                },
                _ => {
                    return Err(
                        "ManuallyDrop::drop requires an exclusive canonical wrapper borrow".into(),
                    );
                }
            },
        };
        wrapper.validate(&plan.catalog, &plan.budget)?;
        self.facts().validate_ty_metadata_with_budget(
            &ir::Ty::ManuallyDrop(wrapper.clone()),
            &plan.budget,
        )?;
        let owner = self
            .declaration
            .ok_or("unsupported: ManuallyDrop operation outside original callable")?;
        let source = ManuallyDropSourceOperation::mint(
            plan,
            owner,
            call,
            ir::ExprId(self.exprs.len()),
            value,
            &plan.budget,
        )?;
        let (kind, ty) = match call.operation {
            ManuallyDropOperation::New => (
                ir::ExprKind::ManuallyDrop {
                    ty: wrapper.clone(),
                    value,
                    source,
                },
                ir::Ty::ManuallyDrop(wrapper),
            ),
            ManuallyDropOperation::IntoInner => {
                let ty = wrapper.argument().clone();
                (
                    ir::ExprKind::ManuallyDropIntoInner {
                        ty: wrapper,
                        value,
                        source,
                    },
                    ty,
                )
            }
            ManuallyDropOperation::Drop => (
                ir::ExprKind::ManuallyDropDrop {
                    ty: wrapper,
                    value,
                    source,
                },
                ir::Ty::Unit,
            ),
        };
        Ok((
            kind,
            Flow {
                ty,
                completes: input.completes,
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn original_drop(plan: &PlanCtx, owner: ir::DeclId) -> SourceManuallyDropCall {
        let function = plan.registry.source_callable(owner).unwrap();
        let mut found = None;
        let mut inspect = |node: &DepthNode<'_>, _: usize| -> Result<(), String> {
            if let DepthNode::Expr(Expr::ManuallyDrop(call)) = node
                && call.operation == ManuallyDropOperation::Drop
            {
                found = Some(call.as_ref().clone());
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
        found.unwrap()
    }

    #[test]
    fn manually_drop_original_operation_rejects_foreign_site_owner_and_unsafe_enclosure() {
        let text = "fn dispose(x:&mut core::mem::ManuallyDrop<i32>){unsafe{core::mem::ManuallyDrop::drop(x);}}fn main(){}";
        let source = parse(text).unwrap();
        let budget = ir::PlanBudget::default();
        let plan = PlanCtx::from_program(&source, &budget).unwrap();
        let owner = plan.function_decl(0).unwrap();
        let call = original_drop(&plan, owner);
        let proof = ManuallyDropSourceOperation::mint(
            &plan,
            owner,
            &call,
            ir::ExprId(3),
            ir::ExprId(2),
            &budget,
        )
        .unwrap();
        proof.validate(&plan.catalog, &budget).unwrap();
        proof.replay_original(&plan, &budget, true).unwrap();
        proof
            .validate_site(
                owner,
                plan.catalog.id(tr::CatalogItem::ManuallyDropDrop),
                ir::ExprId(3),
                ir::ExprId(2),
            )
            .unwrap();
        assert!(
            proof
                .validate_site(owner, proof.declaration(), ir::ExprId(4), ir::ExprId(2))
                .is_err()
        );
        assert!(
            proof
                .validate_site(owner, proof.declaration(), ir::ExprId(3), ir::ExprId(1))
                .is_err()
        );
        assert!(
            proof
                .validate_site(
                    plan.function_decl(1).unwrap(),
                    proof.declaration(),
                    ir::ExprId(3),
                    ir::ExprId(2)
                )
                .is_err()
        );
        let mut forged = proof.as_ref().clone();
        forged.enclosure = None;
        assert!(forged.validate(&plan.catalog, &budget).is_err());
        forged.enclosure = Some(UnsafeEnclosure::Function);
        assert!(forged.replay_original(&plan, &budget, true).is_err());
        forged = proof.as_ref().clone();
        forged.declaration = plan.catalog.id(tr::CatalogItem::ManuallyDropNew);
        assert!(forged.validate(&plan.catalog, &budget).is_err());
        let other = parse(text).unwrap();
        let other_budget = ir::PlanBudget::default();
        let foreign = PlanCtx::from_program(&other, &other_budget).unwrap();
        assert!(
            proof
                .replay_original(&foreign, &other_budget, true)
                .is_err()
        );
        let foreign_call = original_drop(&foreign, foreign.function_decl(0).unwrap());
        assert!(
            ManuallyDropSourceOperation::mint(
                &plan,
                owner,
                &foreign_call,
                ir::ExprId(3),
                ir::ExprId(2),
                &budget
            )
            .is_err()
        );
    }
}
