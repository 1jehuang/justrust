//! Immutable symbolic source templates for graph nominal realization.
use super::*;

impl PlanCtx {
    pub(crate) fn nominal_realization_template(
        &self,
        declaration: ir::DeclId,
    ) -> Result<ir::Ty, String> {
        let info = self.registry.declaration(declaration)?;
        if !matches!(info.kind, ir::DeclKind::Struct | ir::DeclKind::Enum) {
            return Err("nominal realization template requires a struct or enum".into());
        }
        // Planning already registered the canonical symbolic declaration schema.
        // Read that exact schema rather than introducing a second interning path.
        self.nominal_schema(declaration, &self.budget)
            .map(|(_, schema)| schema)
    }
}
