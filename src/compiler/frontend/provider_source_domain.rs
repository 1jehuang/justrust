//! Source-side construction of the same canonical domain decoded by traits.
//! Draft hook: call for the selected provider before sealing its SchemaDecl.
use super::*;

impl DeclRegistry {
    /// Resolve all domains against the immutable authored ownership inventory
    /// before replacing any row. This makes nested provider order irrelevant.
    pub(super) fn install_closure_provider_domains(
        &mut self,
        budget: &ir::PlanBudget,
    ) -> Result<(), String> {
        let mut domains = Vec::new();
        for declaration in &self.declarations {
            if self.source_callable(declaration.id).is_none() {
                continue;
            }
            if let Some(domain) = self.closure_provider_source_domain(declaration.id, budget)? {
                budget.retain_bytes(std::mem::size_of::<(ir::DeclId, Arc<ir::GenericEnv>)>())?;
                domains.push((declaration.id, domain));
            }
        }
        for (id, domain) in domains {
            let info = self
                .declarations
                .get_mut(id.index as usize)
                .filter(|info| info.id == id)
                .ok_or("closure source domain lost its canonical declaration")?;
            info.env = domain;
        }
        Ok(())
    }

    pub(super) fn closure_provider_source_domain(
        &self,
        provider: ir::DeclId,
        budget: &ir::PlanBudget,
    ) -> Result<Option<Arc<ir::GenericEnv>>, String> {
        let mut current = provider;
        let mut adapters = Vec::new();
        let base = loop {
            budget.check_depth(adapters.len())?;
            budget.charge_work(1)?;
            let info = self.declaration(current)?;
            let Some(function) = self.source_callable(current) else {
                if adapters.is_empty() {
                    return Ok(None);
                }
                return Err("closure source domain has no retained callable".into());
            };
            let key = name_scope::path_key(self.names.as_ref(), &function.name, Namespace::Type)?;
            let Some(source) = self.program.closure_sources.get(&key) else {
                if adapters.is_empty() {
                    return Ok(None);
                }
                break info.env.clone();
            };
            if source.stored.is_none() {
                if adapters.is_empty() {
                    return Ok(None);
                }
                break info.env.clone();
            }
            if info.param_env.binder().declaration != current || info.param_env.arity() != 3 {
                return Err("closure adapter own source binder changed".into());
            }
            if adapters
                .iter()
                .any(|scope: &Arc<ir::ParamEnv>| scope.binder() == info.param_env.binder())
            {
                return Err("closure source ownership cycle".into());
            }
            budget.retain_bytes(std::mem::size_of::<Arc<ir::ParamEnv>>())?;
            adapters.push(info.param_env.clone());
            current = self.closure_source_owner(source)?;
        };
        let capacity = base
            .scopes()
            .len()
            .checked_add(adapters.len())
            .ok_or("closure inherited scope capacity overflow")?;
        budget.retain_bytes(
            capacity
                .checked_mul(std::mem::size_of::<Arc<ir::ParamEnv>>())
                .ok_or("closure inherited scope storage overflow")?,
        )?;
        let mut scopes = Vec::with_capacity(capacity);
        scopes.extend(base.scopes().iter().cloned());
        scopes.extend(adapters.into_iter().rev());
        Ok(Some(Arc::new(ir::GenericEnv::new(&scopes, budget)?)))
    }
}
