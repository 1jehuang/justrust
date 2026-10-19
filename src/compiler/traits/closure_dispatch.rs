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
        let adapter = self.decode_closure_adapter(target)?;
        let receiver = match adapter.mode {
            ir::ClosureKind::Fn => ReceiverMode::Borrowed {
                kind: ir::BorrowKind::Shared,
            },
            ir::ClosureKind::FnMut => ReceiverMode::Borrowed {
                kind: ir::BorrowKind::Mut,
            },
            ir::ClosureKind::FnOnce => ReceiverMode::Value,
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
        let adapter = self.world.decode_closure_adapter(target)?;
        if adapter.mode != mode {
            return Ok(false);
        }
        self.world
            .validate_closure_metadata(adapter.closure, self.env())?;
        let original = self.world.closure_target(closure, mode)?;
        if original.type_args.len() != target.type_args.len()
            || target.type_args.len() != self.env().total_arity()
        {
            return Err(FactsError::invalid(
                self.budget(),
                "closure effect lookup changed provider domain arity",
            ));
        }
        for (raw, actual) in original.type_args.iter().zip(target.type_args.iter()) {
            self.world
                .validate_rhs_substitution(raw, actual, self.env(), &target.type_args, 0)?;
        }
        Ok(true)
    }
}
