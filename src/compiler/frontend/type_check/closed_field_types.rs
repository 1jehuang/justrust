//! Constructor fields declared through an associated projection
//! (`Cow::Owned(<B as ToOwned>::Owned)`) are checked against the projection's
//! normal form once the nominal application closes it. The facts normalizer
//! leaves projection-free and still-symbolic (generic) fields unchanged.
use super::*;

impl Checker<'_> {
    pub(super) fn constructor_field_type(&self, field: &ir::Ty) -> Result<ir::Ty, String> {
        let caller = self.facts();
        let Some(facts) = caller.validated() else {
            return Ok(field.clone());
        };
        let plan = self.signatures.plan()?;
        facts
            .normalize_substituted_projection_type(field, &plan.budget)
            .map_err(|error| error.into_legacy_diagnostic(&plan.budget))
    }
}
