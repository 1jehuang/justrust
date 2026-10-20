//! Canonical native iterator declarations and authoritative associated items.
//! This reads the already-published plan/world. It never manufactures an Item
//! from a receiver spelling, strips a reference, or caches a side-slot result.
use super::*;

/// Called only by the existing declaration-registration loop. Canonical source
/// identity is rooted in the embedded prelude module, never a user receiver name.
pub(super) fn native_iterator_slot(
    program: &Program,
    declaration: &TraitDecl,
) -> Result<Option<usize>, String> {
    let Some(root) = program.prelude_module else {
        return Ok(None);
    };
    if program
        .modules
        .parent(declaration.module)
        .map_err(module_diagnostic)?
        != Some(root)
    {
        return Ok(None);
    }
    let Some(module_name) = program
        .modules
        .name(declaration.module)
        .map_err(module_diagnostic)?
    else {
        return Ok(None);
    };
    if program
        .names
        .spelling(module_name)
        .map_err(native_adapter::name_diagnostic)?
        != "iter"
    {
        return Ok(None);
    }
    let name = *declaration
        .name
        .segments()
        .last()
        .ok_or("contract: empty iterator trait path")?;
    Ok(
        match program
            .names
            .spelling(name)
            .map_err(native_adapter::name_diagnostic)?
        {
            "Iterator" => Some(0),
            "IntoIterator" => Some(1),
            _ => None,
        },
    )
}

impl PlanCtx {
    pub fn iterator_trait_id(&self) -> Result<Option<ir::DeclId>, String> {
        self.budget.charge_work(1)?;
        Ok(self.registry.native_iterator_traits[0])
    }

    pub fn into_iterator_trait_id(&self) -> Result<Option<ir::DeclId>, String> {
        self.budget.charge_work(1)?;
        Ok(self.registry.native_iterator_traits[1])
    }

    /// Canonical declaration handles, including the retained source identifier.
    pub fn into_iterator_method(
        &self,
    ) -> Result<Option<(ir::DeclId, AstIdent, ir::DeclId)>, String> {
        self.iterator_method(self.into_iterator_trait_id()?, "into_iter")
    }

    pub fn iterator_next_method(
        &self,
    ) -> Result<Option<(ir::DeclId, AstIdent, ir::DeclId)>, String> {
        self.iterator_method(self.iterator_trait_id()?, "next")
    }

    fn iterator_method(
        &self,
        owner: Option<ir::DeclId>,
        method: &str,
    ) -> Result<Option<(ir::DeclId, AstIdent, ir::DeclId)>, String> {
        let Some(owner) = owner else { return Ok(None) };
        let info = self.registry.declaration(owner)?;
        let source = self.registry.source_trait(owner)?;
        let declarations = self
            .registry
            .trait_items
            .get(info.source_index)
            .ok_or("canonical iterator trait has no method declaration inventory")?;
        let mut found = None;
        for (index, item) in source.items.iter().enumerate() {
            self.budget.charge_work(1)?;
            if self
                .program
                .names
                .spelling(item.signature.name)
                .map_err(native_adapter::name_diagnostic)?
                != method
            {
                continue;
            }
            let declaration = *declarations
                .get(index)
                .ok_or("canonical iterator method declaration inventory mismatch")?;
            if found
                .replace((owner, item.signature.name, declaration))
                .is_some()
            {
                return Err("duplicate canonical iterator method".into());
            }
        }
        found
            .map(Some)
            .ok_or_else(|| format!("canonical iterator declaration has no {method}"))
    }

    /// The exact receiver's IntoIterator::Item, under the caller's authority.
    /// None means the native trait is absent or membership was not proved.
    /// Unsupported normalization, malformed metadata and exhausted resources
    /// remain errors rather than becoming a made-up element type.
    pub fn iterator_item_type(
        &self,
        receiver: &ir::Ty,
        caller: ir::FactsCtx<'_>,
    ) -> Result<Option<ir::Ty>, String> {
        self.iterator_associated_type(receiver, caller, "Item")
    }

    /// Normalize IntoIterator::IntoIter for the exact borrowed or owned receiver.
    pub fn iterator_into_iter_type(
        &self,
        receiver: &ir::Ty,
        caller: ir::FactsCtx<'_>,
    ) -> Result<Option<ir::Ty>, String> {
        self.iterator_associated_type(receiver, caller, "IntoIter")
    }

    fn iterator_associated_type(
        &self,
        receiver: &ir::Ty,
        caller: ir::FactsCtx<'_>,
        item_name: &str,
    ) -> Result<Option<ir::Ty>, String> {
        let Some(declaration) = self.into_iterator_trait_id()? else {
            return Ok(None);
        };
        let facts = caller
            .validated()
            .ok_or("iterator associated item lookup requires validated caller facts")?;
        if !self.budget.same_authority(facts.budget())
            || !Arc::ptr_eq(facts.world(), &self.trait_world()?)
        {
            return Err("iterator associated item changed caller authority".into());
        }
        caller.validate_ty_with_budget(receiver, &self.budget)?;
        let source = self.registry.source_trait(declaration)?;
        let mut item = None;
        for associated in &source.associated_types {
            self.budget.charge_work(1)?;
            if self
                .program
                .names
                .spelling(associated.name)
                .map_err(native_adapter::name_diagnostic)?
                == item_name
                && item
                    .replace(self.registry.associated_type_item(
                        declaration,
                        associated.name,
                        &self.budget,
                    )?)
                    .is_some()
            {
                return Err("duplicate canonical IntoIterator::Item".into());
            }
        }
        let item =
            item.ok_or_else(|| format!("canonical IntoIterator declaration has no {item_name}"))?;
        self.budget
            .retain_bytes(std::mem::size_of::<ir::AssociatedTypeProjection>())?;
        let goal = ir::TraitRef {
            declaration,
            self_ty: receiver.clone(),
            args: Arc::from([]),
        };
        match facts
            .service()
            .prove_goal(&ir::Predicate::Implements(goal.clone()), facts)
            .map_err(|error| error.into_legacy_diagnostic(&self.budget))?
        {
            tr::ProofOutcome::NotProven => return Ok(None),
            tr::ProofOutcome::Proved(_) => {}
        }
        let projection = ir::Ty::AssociatedType(Arc::new(ir::AssociatedTypeProjection {
            trait_ref: goal,
            item,
        }));
        // normalize_ty also honors caller-owned associated equality predicates.
        let item = facts
            .world()
            .normalize_ty(&projection, facts)
            .map_err(|error| error.into_legacy_diagnostic(&self.budget))?;
        caller.validate_ty_with_budget(&item, &self.budget)?;
        Ok(Some(item))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(native: bool) -> PlanCtx {
        let mut program = parse("mod library { pub mod iter { pub trait IntoIterator { type Item; type IntoIter; } pub trait Iterator { type Item; } } } impl library::iter::IntoIterator for i32 { type Item = u8; type IntoIter = u32; } impl library::iter::IntoIterator for &i32 { type Item = u64; type IntoIter = u64; } impl library::iter::IntoIterator for &mut i32 { type Item = u16; type IntoIter = u16; } fn main() {}").unwrap();
        if native {
            program.prelude_module = program.modules.parent(program.traits[0].module).unwrap();
        }
        PlanCtx::from_program(&program, &ir::PlanBudget::default()).unwrap()
    }

    #[test]
    fn absent_native_module_does_not_treat_user_names_as_iterator_authority() {
        let plan = plan(false);
        assert_eq!(plan.iterator_trait_id().unwrap(), None);
        assert_eq!(plan.into_iterator_trait_id().unwrap(), None);
        assert_eq!(
            plan.iterator_item_type(&ir::Ty::Int(ir::IntTy::I32), ir::FactsCtx::Concrete)
                .unwrap(),
            None
        );
    }

    #[test]
    fn exact_receiver_projection_preserves_shared_reference_membership() {
        let plan = plan(true);
        let facts = plan.callable_facts(plan.function_decl(0).unwrap()).unwrap();
        let caller = ir::FactsCtx::Validated(&facts);
        assert!(plan.iterator_trait_id().unwrap().is_some());
        assert_eq!(
            plan.iterator_item_type(&ir::Ty::Int(ir::IntTy::I32), caller)
                .unwrap(),
            Some(ir::Ty::Int(ir::IntTy::U8))
        );
        let shared = ir::Ty::Ref {
            kind: ir::BorrowKind::Shared,
            referent: Arc::new(ir::Ty::Int(ir::IntTy::I32)),
        };
        assert_eq!(
            plan.iterator_item_type(&shared, caller).unwrap(),
            Some(ir::Ty::Int(ir::IntTy::U64))
        );
        let mutable = ir::Ty::Ref {
            kind: ir::BorrowKind::Mut,
            referent: Arc::new(ir::Ty::Int(ir::IntTy::I32)),
        };
        assert_eq!(
            plan.iterator_item_type(&mutable, caller).unwrap(),
            Some(ir::Ty::Int(ir::IntTy::U16))
        );
        assert_eq!(
            plan.iterator_into_iter_type(&shared, caller).unwrap(),
            Some(ir::Ty::Int(ir::IntTy::U64))
        );
        assert_eq!(
            plan.iterator_into_iter_type(&mutable, caller).unwrap(),
            Some(ir::Ty::Int(ir::IntTy::U16))
        );
        assert_eq!(
            plan.iterator_into_iter_type(&ir::Ty::Int(ir::IntTy::I32), caller)
                .unwrap(),
            Some(ir::Ty::Int(ir::IntTy::U32))
        );
        assert_eq!(
            plan.iterator_item_type(&ir::Ty::Bool, caller).unwrap(),
            None
        );
    }

    #[test]
    fn canonical_methods_return_retained_source_and_declaration_identity() {
        let mut program = parse("mod library { pub mod iter { pub trait IntoIterator { type IntoIter; fn into_iter(self) -> Self::IntoIter; } pub trait Iterator { type Item; fn next(&mut self) -> Self::Item; } } } fn main() {}").unwrap();
        program.prelude_module = program.modules.parent(program.traits[0].module).unwrap();
        let plan = PlanCtx::from_program(&program, &ir::PlanBudget::default()).unwrap();
        for (method, expected) in [
            (plan.into_iterator_method().unwrap(), "into_iter"),
            (plan.iterator_next_method().unwrap(), "next"),
        ] {
            let (owner, name, declaration) = method.unwrap();
            assert_eq!(plan.program.names.spelling(name).unwrap(), expected);
            assert_eq!(
                plan.registry.declaration(owner).unwrap().kind,
                ir::DeclKind::Trait
            );
            assert_eq!(
                plan.registry.declaration(declaration).unwrap().kind,
                ir::DeclKind::TraitItem
            );
            assert!(
                name_scope::ident_eq(
                    plan.program.names.as_ref(),
                    name,
                    plan.registry.source_trait(owner).unwrap().items[0]
                        .signature
                        .name,
                    LookupClass::Member
                )
                .unwrap()
            );
        }
        assert!(self::plan(false).into_iterator_method().unwrap().is_none());
    }

    #[test]
    fn foreign_facts_and_resource_failures_do_not_become_absent_items() {
        let plan = plan(true);
        let foreign = self::plan(true);
        let facts = foreign
            .callable_facts(foreign.function_decl(0).unwrap())
            .unwrap();
        assert!(
            plan.iterator_item_type(
                &ir::Ty::Int(ir::IntTy::I32),
                ir::FactsCtx::Validated(&facts)
            )
            .unwrap_err()
            .contains("authority")
        );
        let remaining = ir::PlanLimits::default().work - plan.budget.usage_snapshot().work;
        plan.budget.charge_work(remaining).unwrap();
        assert!(plan.into_iterator_trait_id().is_err());
    }
}
