use super::*;

impl<'a, 'b, 's> Solver<'a, 'b, 's> {
    /// Positional patterns use the same syntax for tuple structs and enum
    /// variants. Resolve the entire canonical type path before splitting a
    /// variant owner, just as the authoritative pattern checker does.
    pub(super) fn positional_pattern(
        &mut self,
        name: &AstPath,
        fields: &[(FieldSelector, &'s Pattern)],
        term: Term,
    ) -> Result<(), String> {
        let plan = self.checker.signatures.plan()?;
        let key = self.checker.path_key(name, Namespace::Type)?;
        if plan.registry.nominals.contains_key(&key) || self.checker.self_path(name)? {
            self.nominal_pattern(name, None, fields, term)
        } else {
            let (owner, variant) = self.checker.split_variant(name)?;
            self.nominal_pattern(&owner, Some(variant), fields, term)
        }
    }
}
