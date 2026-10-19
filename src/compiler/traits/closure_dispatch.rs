//! Mode-qualified identities for immutable checked closure body proofs.
use super::*;

impl ValidatedTraitWorld {
    // Callers first discharge the exact target, including its checked closure
    // metadata and supported capability. The adapter's source declaration is
    // receiver-generic, so its instantiated first actual defines the mode.
    pub(super) fn closure_dispatch_receiver(
        &self,
        target: &CallTarget,
    ) -> Result<Option<ReceiverMode>> {
        if self
            .schema
            .closure_owner_site(target.declaration)?
            .is_none()
        {
            return Ok(None);
        }
        let receiver = match target.type_args.first() {
            Some(Ty::Ref { kind, referent }) if matches!(referent.as_ref(), Ty::Closure(_)) => {
                ReceiverMode::Borrowed { kind: *kind }
            }
            Some(Ty::Closure(_)) => ReceiverMode::Value,
            _ => {
                return Err(FactsError::invalid(
                    &self.budget,
                    "closure adapter has no canonical receiver",
                ));
            }
        };
        Ok(Some(receiver))
    }
}

impl ValidatedFacts {
    pub(in crate::compiler) fn same_body_dispatch(&self, other: &Self) -> Result<bool> {
        if self.declaration() != other.declaration() {
            return Ok(false);
        }
        match (&self.closure, &other.closure) {
            (None, None) => Ok(true),
            (Some(left), Some(right)) => {
                if !self
                    .world
                    .types_equal(&[Ty::Closure(left.clone())], &[Ty::Closure(right.clone())])?
                {
                    return Err(FactsError::invalid(
                        self.budget(),
                        "closure body proofs changed canonical environment metadata",
                    ));
                }
                Ok(self.closure_mode == other.closure_mode)
            }
            _ => Err(FactsError::invalid(
                self.budget(),
                "closure body proof lost checked environment authority",
            )),
        }
    }
    pub(in crate::compiler) fn body_dispatch_matches(&self, target: &CallTarget) -> Result<bool> {
        if target.declaration != self.declaration() {
            return Ok(false);
        }
        let Some(closure) = &self.closure else {
            return Ok(true);
        };
        let Some(mode) = self.closure_mode else {
            return Err(FactsError::invalid(
                self.budget(),
                "closure body proof has no receiver capability",
            ));
        };
        let original = self.world.closure_target(closure, mode)?;
        self.world
            .types_equal(&original.type_args, &target.type_args)
    }
}
