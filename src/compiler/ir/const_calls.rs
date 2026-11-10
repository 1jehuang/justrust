use super::*;

impl Callee {
    /// Visit value-type operands without treating a constant coordinate as a
    /// runtime type. The complete ordered metadata remains available for
    /// source replay and equality, and exact declaration kinds are checked
    /// before any constant leaf can be omitted from this value-only walk.
    pub(in crate::compiler) fn for_each_value_metadata(
        &self,
        facts: FactsCtx<'_>,
        mut visit: impl FnMut(&Ty) -> Result<(), String>,
    ) -> Result<(), String> {
        if let Some(authority) = facts.validated() {
            authority
                .budget()
                .charge_work(self.metadata_types().count().saturating_mul(2))?;
        }
        let mut has_constants = false;
        for ty in self.metadata_types() {
            has_constants |= ty.is_const_argument_leaf(Some(facts))?;
        }
        if has_constants {
            let authority = facts
                .validated()
                .ok_or("const call metadata requires authoritative declaration facts")?;
            authority
                .validate_callee_metadata(self)
                .map_err(|error| error.into_legacy_diagnostic(authority.budget()))?;
        }
        for ty in self.metadata_types() {
            if !ty.is_const_argument_leaf(Some(facts))? {
                visit(ty)?;
            }
        }
        Ok(())
    }
}
