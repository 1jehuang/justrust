//! Source identities used by builtin closure obligations. A user trait merely
//! spelled `Fn` never receives the compiler's callable implementation rule.
use super::*;

impl SchemaAdapter {
    pub(in crate::compiler::frontend) fn source_closure_trait_kind(
        &self,
        declaration: ir::DeclId,
    ) -> Result<Option<ir::ClosureKind>, tr::FactsError> {
        let info = self
            .registry
            .declaration(declaration)
            .map_err(|e| self.error(e))?;
        if info.kind != ir::DeclKind::Trait || !self.registry.program.in_prelude(info.path.module) {
            return Ok(None);
        }
        let DeclItemName::Source(name) = info.path.item else {
            return Ok(None);
        };
        Ok(
            match self
                .registry
                .names
                .spelling(name)
                .map_err(|e| self.error(native_adapter::name_diagnostic(e)))?
            {
                "Fn" => Some(ir::ClosureKind::Fn),
                "FnMut" => Some(ir::ClosureKind::FnMut),
                "FnOnce" => Some(ir::ClosureKind::FnOnce),
                _ => None,
            },
        )
    }

    pub(in crate::compiler::frontend) fn source_closure_owner_site(
        &self,
        provider: ir::DeclId,
    ) -> Result<Option<(ir::DeclId, u32)>, tr::FactsError> {
        let info = self
            .registry
            .declaration(provider)
            .map_err(|e| self.error(e))?;
        if info.kind != ir::DeclKind::Function {
            return Ok(None);
        }
        let Some(function) = self.registry.program.functions.get(info.source_index) else {
            return Ok(None);
        };
        let key = name_scope::path_key(
            self.registry.names.as_ref(),
            &function.name,
            Namespace::Type,
        )
        .map_err(|e| self.error(e))?;
        let Some(source) = self.registry.program.closure_sources.get(&key) else {
            return Ok(None);
        };
        let Some(stored) = &source.stored else {
            return Ok(None);
        };
        let parent = source
            .parent
            .as_ref()
            .ok_or_else(|| self.error("stored closure lacks its original parent".to_owned()))?;
        let key =
            name_scope::path_key(self.registry.names.as_ref(), &parent.name, Namespace::Value)
                .map_err(|e| self.error(e))?;
        let owner = *self
            .registry
            .functions
            .get(&key)
            .ok_or_else(|| self.error("stored closure parent is not a callable".to_owned()))?;
        Ok(Some((owner, stored.site)))
    }
}
