//! One canonical decoder for a closure provider's inherited owner domain.
//!
//! Source-schema construction retains the same parent scopes and predicates.
//! Ordinary shape admission remains independently mandatory. No consumer may
//! infer the prefix from target length.
use super::*;

pub(super) struct ClosureAdapterActuals<'a> {
    pub closure: &'a Arc<ir::ClosureTy>,
    pub mode: ir::ClosureKind,
}

impl ValidatedTraitWorld {
    /// All downstream consumers decode the registered source domain, rather
    /// than assuming receiver actual zero after parent arguments are added.
    pub(in crate::compiler) fn closure_adapter_environment(
        &self,
        target: &CallTarget,
        caller: &GenericEnv,
    ) -> Result<(Arc<ir::ClosureTy>, ir::ClosureKind)> {
        let adapter = self.decode_closure_adapter(target)?;
        self.validate_closure_metadata(adapter.closure, caller)?;
        Ok((adapter.closure.clone(), adapter.mode))
    }

    /// Authenticate a closed or symbolic instance against the exact retained
    /// parent template. This does not replace the parent body's publication
    /// replay, which the frontend performs independently before calling here.
    pub(in crate::compiler) fn validate_closure_instance_substitution(
        &self,
        original: &ir::ClosureTy,
        target: &CallTarget,
        caller: &GenericEnv,
    ) -> Result<ir::ClosureKind> {
        let adapter = self.decode_closure_adapter(target)?;
        self.validate_closure_metadata(adapter.closure, caller)?;
        let header = self.callable_header(target.declaration)?;
        self.validate_closure_rhs_substitution(
            original,
            adapter.closure,
            &header.schema.env,
            &target.type_args,
            0,
        )?;
        Ok(adapter.mode)
    }

    pub(super) fn closure_target_in_owner_domain(
        &self,
        closure: &Arc<ir::ClosureTy>,
        mode: ir::ClosureKind,
    ) -> Result<CallTarget> {
        let prefix = self.closure_provider_prefix(closure.provider())?;
        if prefix != closure.owner_args().len() || !closure.supports(mode) {
            return Err(FactsError::invalid(
                &self.budget,
                "closure target changed its canonical owner domain or capability",
            ));
        }
        let receiver = match mode {
            ir::ClosureKind::Fn => Ty::Ref {
                kind: ir::BorrowKind::Shared,
                referent: Arc::new(Ty::Closure(closure.clone())),
            },
            ir::ClosureKind::FnMut => Ty::Ref {
                kind: ir::BorrowKind::Mut,
                referent: Arc::new(Ty::Closure(closure.clone())),
            },
            ir::ClosureKind::FnOnce => Ty::Closure(closure.clone()),
        };
        let count = prefix.checked_add(3).ok_or_else(|| {
            FactsError::invalid(&self.budget, "closure target argument capacity overflow")
        })?;
        retain::<Ty>(&self.budget, count)?;
        let mut type_args = Vec::with_capacity(count);
        type_args.extend(closure.owner_args().iter().cloned());
        type_args.push(receiver);
        type_args.push(self.closure_arguments(closure)?);
        type_args.push(closure.result().clone());
        let target = CallTarget {
            declaration: closure.provider(),
            type_args: type_args.into(),
        };
        self.decode_closure_adapter(&target)?;
        Ok(target)
    }

    pub(super) fn closure_provider_prefix(&self, provider: DeclId) -> Result<usize> {
        let invalid = |message| FactsError::invalid(&self.budget, message);
        let (owner, _) = self
            .schema
            .closure_owner_site(provider)?
            .ok_or_else(|| invalid("closure adapter has no canonical source parent"))?;
        let owner = self.info(owner)?;
        let header = self.callable_header(provider)?;
        let parent_scopes = owner.env.scopes();
        let provider_scopes = header.schema.env.scopes();
        let Some((own, inherited)) = provider_scopes.split_last() else {
            return Err(invalid("closure adapter lacks its own binder scope"));
        };
        if inherited.len() != parent_scopes.len()
            || own.binder().declaration != provider
            || own.arity() != 3
        {
            return Err(invalid("closure adapter changed its source owner domain"));
        }
        for (original, inherited) in parent_scopes.iter().zip(inherited) {
            charged(&self.budget, self.budget.charge_work(1))?;
            if original.binder() != inherited.binder() || original.arity() != inherited.arity() {
                return Err(invalid(
                    "closure adapter reordered or replaced a parent binder",
                ));
            }
        }
        Ok(owner.env.total_arity())
    }

    pub(super) fn decode_closure_adapter<'a>(
        &self,
        target: &'a CallTarget,
    ) -> Result<ClosureAdapterActuals<'a>> {
        let invalid = |message| FactsError::invalid(&self.budget, message);
        let (owner, site) = self
            .schema
            .closure_owner_site(target.declaration)?
            .ok_or_else(|| invalid("closure call target has no retained source site"))?;
        let prefix = self.closure_provider_prefix(target.declaration)?;
        let expected = prefix
            .checked_add(3)
            .ok_or_else(|| invalid("closure adapter domain arity overflow"))?;
        if target.type_args.len() != expected {
            return Err(invalid(
                "closure adapter actuals do not fill its canonical domain",
            ));
        }
        let (owner_arguments, adapter_arguments) = target.type_args.split_at(prefix);
        let [receiver, arguments, result] = adapter_arguments else {
            return Err(invalid("closure adapter own argument domain changed"));
        };
        let (environment, mode) = match receiver {
            Ty::Ref { kind, referent } => (
                referent.as_ref(),
                match kind {
                    ir::BorrowKind::Shared => ir::ClosureKind::Fn,
                    ir::BorrowKind::Mut => ir::ClosureKind::FnMut,
                },
            ),
            other => (other, ir::ClosureKind::FnOnce),
        };
        let Ty::Closure(closure) = environment else {
            return Err(invalid("closure adapter receiver is not its environment"));
        };
        if closure.owner() != owner
            || closure.site() != site
            || closure.provider() != target.declaration
            || !closure.supports(mode)
            || closure.owner_args().len() != owner_arguments.len()
        {
            return Err(invalid(
                "closure adapter changed owner, site, provider or capability",
            ));
        }
        for (expected, actual) in closure.owner_args().iter().zip(owner_arguments) {
            if !self.eq_ty(expected, actual, 0)? {
                return Err(invalid(
                    "closure target prefix differs from its sealed owner actuals",
                ));
            }
        }
        if !self.eq_ty(arguments, &self.closure_arguments(closure)?, 0)?
            || !self.eq_ty(result, closure.result(), 0)?
        {
            return Err(invalid(
                "closure target tuple or result differs from its sealed signature",
            ));
        }
        Ok(ClosureAdapterActuals { closure, mode })
    }
}
