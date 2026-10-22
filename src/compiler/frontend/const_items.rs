//! Immutable declaration lookup for original-source scalar constants.
//!
//! These helpers expose identities and borrowed source only. Evaluated values
//! must be obtained from CONST_VALUE graph handles, never from the registry.
use super::*;

impl PlanCtx {
    pub(in crate::compiler) fn constant_unresolved_value(
        &self,
        path: &AstPath,
    ) -> Result<String, String> {
        let key = name_scope::path_key(self.registry.names.as_ref(), path, Namespace::Value)?;
        if self.registry.functions.contains_key(&key) {
            return Ok("E0308: function item is not a scalar constant value".into());
        }
        let name = path
            .display(self.program.names.as_ref())
            .map_err(native_adapter::name_diagnostic)?;
        let length = name
            .as_str()
            .len()
            .checked_add(64)
            .ok_or("constant name diagnostic overflow")?;
        self.budget.charge_diagnostic_bytes(length)?;
        self.budget.retain_bytes(length)?;
        Ok(format!(
            "E0425: cannot find value `{}` in constant initializer",
            name.as_str()
        ))
    }
    pub(in crate::compiler) fn constant_key_decl(
        &self,
        index: usize,
    ) -> Result<ir::DeclId, String> {
        let snapshot = self.registry_snapshot_id()?;
        let id = ir::DeclId {
            snapshot,
            index: u64::try_from(index).map_err(
                |_| "contract: constant graph key is not a current constant declaration",
            )?,
        };
        self.registry
            .declaration(id)
            .ok()
            .filter(|info| info.kind == ir::DeclKind::Const)
            .map(|info| info.id)
            .ok_or_else(|| {
                "contract: constant graph key is not a current constant declaration".into()
            })
    }
    pub(in crate::compiler) fn constant_callable_source(
        &self,
        declaration: ir::DeclId,
    ) -> Result<&Function, String> {
        let info = self.declaration(declaration)?;
        if info.kind != ir::DeclKind::Function {
            return Err("unsupported: non-free call in constant initializer".into());
        }
        self.registry
            .source_callable(declaration)
            .ok_or_else(|| "unsupported: non-source call in constant initializer".into())
    }
    pub(in crate::compiler) fn publish_constant_declaration(
        &self,
        program: &Arc<Program>,
        declaration: ir::DeclId,
        budget: &ir::PlanBudget,
    ) -> Result<super::super::const_source::DeclarationFact, String> {
        if !Arc::ptr_eq(&self.program, program) {
            return Err("contract: constant source graph publication changed".into());
        }
        super::super::const_source::project(
            self,
            declaration,
            self.constant_source(declaration)?,
            budget,
        )
    }

    pub(in crate::compiler) fn constant_callee(
        &self,
        path: &AstPath,
    ) -> Result<ir::DeclId, String> {
        let key = name_scope::path_key(self.registry.names.as_ref(), path, Namespace::Value)?;
        self.registry
            .functions
            .get(&key)
            .copied()
            .ok_or_else(|| "constant initializer call does not resolve to a function".into())
    }

    pub(in crate::compiler) fn constant_count(&self) -> usize {
        self.registry.const_ids.len()
    }

    pub(in crate::compiler) fn constant_decl(&self, index: usize) -> Result<ir::DeclId, String> {
        self.registry
            .const_ids
            .get(index)
            .copied()
            .ok_or_else(|| "contract: constant declaration index outside registry".into())
    }

    pub(in crate::compiler) fn constant_source(
        &self,
        decl: ir::DeclId,
    ) -> Result<&ConstDecl, String> {
        let info = self.declaration(decl)?;
        if info.kind != ir::DeclKind::Const {
            return Err("contract: expected constant declaration identity".into());
        }
        self.program
            .constants
            .get(info.source_index)
            .ok_or_else(|| "contract: constant source index outside publication".into())
    }

    pub(in crate::compiler) fn constant_path(&self, path: &AstPath) -> Result<ir::DeclId, String> {
        let key = name_scope::path_key(self.registry.names.as_ref(), path, Namespace::Value)?;
        self.registry
            .constants
            .get(&key)
            .copied()
            .ok_or_else(|| "contract: resolved constant path absent from registry".into())
    }
}
