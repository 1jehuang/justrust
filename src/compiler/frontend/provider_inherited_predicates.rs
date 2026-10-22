//! Completion inside the existing source-schema builder, before publication.
//! No graph-facing mutable state or new all-program operation is introduced.
//! Intended as a child module of frontend::source_schemas.
use super::*;

impl SourceSchemasBuilder {
    /// Compute every replacement against the same original row inventory.
    /// Applying a replacement during traversal would make nested providers
    /// inherit already-expanded ancestors and duplicate their obligations.
    pub(in crate::compiler::frontend) fn complete_closure_predicates(
        &mut self,
        registry: &super::super::DeclRegistry,
        budget: &ir::PlanBudget,
    ) -> Result<(), String> {
        let mut replacements = Vec::new();
        for (id, _) in self.functions.values.iter().flatten() {
            budget.charge_work(1)?;
            if registry
                .closure_provider_source_domain(*id, budget)?
                .is_some()
            {
                let predicates = self.closure_inherited_predicates(*id, registry, budget)?;
                budget.retain_bytes(std::mem::size_of::<(ir::DeclId, Arc<[ir::Predicate]>)>())?;
                replacements.push((*id, predicates));
            }
        }
        for (id, predicates) in replacements {
            let position = table_position(&self.functions.values, id).ok();
            let row = position
                .and_then(|position| self.functions.values[position].as_mut())
                .filter(|(stored, _)| *stored == id)
                .ok_or("closure predicate replacement lost its canonical declaration")?;
            row.1.value.predicates = predicates;
        }
        Ok(())
    }

    pub(super) fn closure_inherited_predicates(
        &self,
        provider: ir::DeclId,
        registry: &super::super::DeclRegistry,
        budget: &ir::PlanBudget,
    ) -> Result<Arc<[ir::Predicate]>, String> {
        let mut current = provider;
        let mut chain = Vec::new();
        loop {
            budget.check_depth(chain.len())?;
            budget.charge_work(1)?;
            let function = registry
                .source_callable(current)
                .ok_or("closure predicate owner has no source body")?;
            let key = super::super::name_scope::path_key(
                registry.names.as_ref(),
                &function.name,
                super::super::Namespace::Type,
            )?;
            let source = registry
                .program
                .closure_sources
                .get(&key)
                .filter(|source| source.stored.is_some());
            let Some(source) = source else {
                break;
            };
            if chain.contains(&current) {
                return Err("closure predicate owner cycle".into());
            }
            budget.retain_bytes(std::mem::size_of::<ir::DeclId>())?;
            chain.push(current);
            current = registry.closure_source_owner(source)?;
        }
        if chain.is_empty() {
            return Err("closure predicate completion requires a retained provider".into());
        }
        // The outer authored owner already has its full canonical schema here,
        // including impl/trait requirements and implicit bounds. Nested provider
        // rows retain their own adapter bounds, not merely their source spelling.
        let base = lookup(&self.functions.values, current, budget)?
            .ok_or("closure parent predicates are not finalized")?;
        let mut capacity = base.value.predicates.len();
        for id in &chain {
            let row = lookup(&self.functions.values, *id, budget)?
                .ok_or("closure provider schema is not finalized")?;
            capacity = capacity
                .checked_add(row.value.predicates.len())
                .ok_or("closure inherited predicate count overflow")?;
        }
        budget.retain_bytes(
            capacity
                .checked_mul(std::mem::size_of::<ir::Predicate>())
                .ok_or("closure inherited predicate storage overflow")?,
        )?;
        let mut rows = Vec::with_capacity(capacity);
        rows.extend(base.value.predicates.iter().cloned());
        for id in chain.into_iter().rev() {
            let row = lookup(&self.functions.values, id, budget)?
                .ok_or("closure inherited provider disappeared")?;
            budget.charge_work(row.value.predicates.len())?;
            rows.extend(row.value.predicates.iter().cloned());
        }
        Ok(rows.into())
    }
}
