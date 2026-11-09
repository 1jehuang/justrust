use super::*;

impl Replay<'_> {
    /// Validate authored UFCS ownership independently of the checked callee's actuals.
    pub(super) fn associated_method_owner(
        &self,
        source: &Type,
        declaration: ir::DeclId,
        reconstructed: &ir::Ty,
    ) -> Result<(), FactsError> {
        if let Type::Apply(path) = source {
            let (_, written) = path
                .application()
                .map_err(|e| source_error(self.budget, &e))?;
            sr::work(self.budget, written.len())?;
            if written.iter().all(|ty| matches!(ty, Type::Infer)) {
                let Some((owner_declaration, owner_actuals)) = nominal_parts(reconstructed) else {
                    return Err(FactsError::invalid(
                        self.budget,
                        "associated method reconstructed owner is not nominal",
                    ));
                };
                if owner_declaration != declaration || written.len() != owner_actuals.len() {
                    return Err(FactsError::invalid(
                        self.budget,
                        "associated method source owner arity changed",
                    ));
                }
                let context = self
                    .original
                    .plan
                    .callable_context(self.original.declaration)
                    .map_err(|e| source_error(self.budget, &e))?;
                let checker = Checker::new(self.signatures, ir::Ty::Unit, context.module);
                let owner_path = path
                    .path(checker.names().map_err(|e| source_error(self.budget, &e))?)
                    .map_err(|e| source_error(self.budget, &e))?;
                let key = checker
                    .path_key(&owner_path, Namespace::Type)
                    .map_err(|e| source_error(self.budget, &e))?;
                if self.original.plan.registry.nominals.get(&key) != Some(&declaration) {
                    return Err(FactsError::invalid(
                        self.budget,
                        "associated method source owner changed",
                    ));
                }
                return Ok(());
            }
        }
        let owner =
            checked_local_ty(source, self.signatures).map_err(|e| source_error(self.budget, &e))?;
        if !self.equal(&owner, reconstructed)? {
            return Err(FactsError::invalid(
                self.budget,
                "associated method explicit source owner changed",
            ));
        }
        Ok(())
    }
}
