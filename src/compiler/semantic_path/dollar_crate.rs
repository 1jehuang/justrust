//! $crate keeps the definition crate token identity, including through forwarding.
use super::*;
use crate::compiler::token::{Token, TokenKind};
use crate::compiler::token_session::{CrateId, SourceFileHandle};

/// An actual admitted token, not an extern spelling or a caller-selected crate.
#[derive(Clone, Copy, Debug)]
pub struct DollarCrateAnchor {
    root: SourceFileHandle,
    crate_id: CrateId,
    token: Token,
    head: Option<(SessionId, u32)>,
}
/// Compact AST transport. The immutable original token proof and its backing
/// charge are shared, so cloning a path never duplicates or unmeters authority.
#[derive(Clone)]
pub struct RetainedDollarCrateAnchor(Arc<RetainedAnchorData>);
struct RetainedAnchorData {
    proof: DollarCrateAnchor,
    _lease: ByteLease,
}
impl fmt::Debug for RetainedDollarCrateAnchor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.proof.fmt(f)
    }
}
impl PartialEq for RetainedDollarCrateAnchor {
    fn eq(&self, other: &Self) -> bool {
        self.0.proof == other.0.proof
    }
}
impl Eq for RetainedDollarCrateAnchor {}
impl Hash for RetainedDollarCrateAnchor {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.proof.hash(state);
    }
}
impl RetainedDollarCrateAnchor {
    pub(super) fn same_occurrence(&self, other: &Self) -> bool {
        self.0.proof.same_occurrence(other.0.proof)
    }
    pub(in crate::compiler) fn bind(
        &self,
        names: &impl NameAccess,
        head: AstIdent,
    ) -> Result<Self, NameError> {
        let proof = self.0.proof.bind(names, head)?;
        if proof.same_occurrence(self.0.proof) {
            return Ok(self.clone());
        }
        Ok(proof.retain(names.session())?)
    }
    pub(in crate::compiler) fn check_head(
        &self,
        names: &impl NameAccess,
        head: Option<AstIdent>,
    ) -> Result<(), NameError> {
        self.0.proof.check_head(names, head)
    }
    pub(in crate::compiler) fn root(
        &self,
        session: &TokenSession,
    ) -> Result<SourceFileHandle, TokenError> {
        self.0.proof.root(session)
    }
}
impl DollarCrateAnchor {
    pub(in crate::compiler) fn retain(
        self,
        session: &TokenSession,
    ) -> Result<RetainedDollarCrateAnchor, TokenError> {
        self.root(session)?;
        let fixed = size_of::<RetainedAnchorData>()
            .checked_add(2 * size_of::<usize>())
            .ok_or(TokenError::IdExhausted)?;
        let lease = session.budget().reserve(Storage::Retained, fixed as u64)?;
        Ok(RetainedDollarCrateAnchor(Arc::new(RetainedAnchorData {
            proof: self,
            _lease: lease,
        })))
    }
    pub(in crate::compiler) fn admitted(
        session: &TokenSession,
        token: Token,
        proof: &crate::compiler::semantic_scope::ExpansionTokenAdmission,
    ) -> Result<Self, crate::compiler::semantic_scope::ScopeError> {
        proof.validate_session(session)?;
        proof.validate_span(session, token.span)?;
        token.validate(session)?;
        let TokenKind::DollarCrate(crate_id) = token.kind else {
            return Err(TokenError::InvalidHandle.into());
        };
        Ok(Self {
            root: session.crate_root(crate_id)?,
            crate_id,
            token,
            head: None,
        })
    }
    pub(super) fn same_occurrence(self, other: Self) -> bool {
        self.root == other.root
            && self.crate_id == other.crate_id
            && self.token == other.token
            && self.head == other.head
    }
    pub(in crate::compiler) fn bind(
        mut self,
        names: &impl NameAccess,
        head: AstIdent,
    ) -> Result<Self, NameError> {
        self.root(names.session())?;
        let identity = (
            super::super::semantic_name::occurrence_owner(names, head)?,
            super::super::semantic_name::occurrence_index(names, head)?,
        );
        if self.head.is_some_and(|old| old != identity) {
            return Err(
                TokenError::InvalidInput("definition-crate path head occurrence changed").into(),
            );
        }
        self.head = Some(identity);
        Ok(self)
    }
    pub(in crate::compiler) fn check_head(
        self,
        names: &impl NameAccess,
        head: Option<AstIdent>,
    ) -> Result<(), NameError> {
        let head = head.ok_or(TokenError::InvalidHandle)?;
        if self.head.is_none() {
            return Err(TokenError::InvalidHandle.into());
        }
        self.bind(names, head)?;
        Ok(())
    }
    pub(in crate::compiler) fn root(
        self,
        session: &TokenSession,
    ) -> Result<SourceFileHandle, TokenError> {
        self.token.validate(session)?;
        if self.token.kind != TokenKind::DollarCrate(self.crate_id)
            || session.crate_root(self.crate_id)? != self.root
            || session.span(self.token.span)?.expansion.is_none()
        {
            return Err(TokenError::InvalidHandle);
        }
        Ok(self.root)
    }
}
// Namespace equality is definition-crate identity. The retained token remains
// separately checked on admission, and does not change semantic cache keys.
impl PartialEq for DollarCrateAnchor {
    fn eq(&self, other: &Self) -> bool {
        self.root == other.root
    }
}
impl Eq for DollarCrateAnchor {}
impl Hash for DollarCrateAnchor {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.root.hash(state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::expansion_budget::ExpansionLimits;
    use crate::compiler::expansion_span::ExpansionRecord;
    use crate::compiler::token::Edition;
    use crate::compiler::token_session::SourceSnapshot;
    use std::path::Path;

    fn fixture(budget: ExpansionBudget) -> (NameStoreBuilder, DollarCrateAnchor, [AstIdent; 2]) {
        let sources = SourceSnapshot::from_text(
            Path::new("dollar-crate-proof.rs"),
            "helper helper",
            &budget,
            crate::compiler::owner_path::standalone_issued(
                &budget,
                crate::compiler::owner_path::OwnerDomain::SourceSnapshot,
            )
            .unwrap(),
        )
        .unwrap();
        let root = sources.file(0).unwrap();
        let original = sources.span(root, 0, 6).unwrap();
        let other = sources.span(root, 7, 13).unwrap();
        let (mut session, cap) = TokenSession::standalone(sources, budget).unwrap();
        let span = session.original_span(original).unwrap();
        let other = session.original_span(other).unwrap();
        let crate_id = session.register_crate_root(root).unwrap();
        let head_span = span;
        let expansion = session
            .add_expansion(ExpansionRecord {
                definition: span,
                call: span,
                parent: None,
                defining_crate: crate_id,
                edition: Edition::E2024,
            })
            .unwrap();
        let span = session
            .with_context(span, session.root_context(), Some(expansion))
            .unwrap();
        let proof = DollarCrateAnchor {
            root,
            crate_id,
            token: Token {
                span,
                kind: TokenKind::DollarCrate(crate_id),
            },
            head: None,
        };
        let mut names = NameStoreBuilder::new(session, cap, Edition::E2024).unwrap();
        let first = names.intern_root("helper", head_span).unwrap();
        let second = names.intern_root("helper", other).unwrap();
        (names, proof, [first, second])
    }

    #[test]
    fn compact_proof_does_not_inflate_ast_paths() {
        assert_eq!(size_of::<RetainedDollarCrateAnchor>(), size_of::<usize>());
        assert!(size_of::<PathAnchor>() <= 2 * size_of::<usize>());
        assert!(size_of::<AstPath>() <= 4 * size_of::<usize>());
    }

    #[test]
    fn retained_proof_charges_actual_arc_storage_until_last_owner() {
        let budget = ExpansionBudget::default();
        let (names, proof, [first, second]) = fixture(budget.clone());
        let before = budget.snapshot().retained_bytes;
        let retained = proof.retain(names.session()).unwrap();
        let fixed = (size_of::<RetainedAnchorData>() + 2 * size_of::<usize>()) as u64;
        assert_eq!(budget.snapshot().retained_bytes - before, fixed);
        let bound = retained.bind(&names, first).unwrap();
        assert_eq!(budget.snapshot().retained_bytes - before, 2 * fixed);
        let again = bound.bind(&names, first).unwrap();
        assert!(Arc::ptr_eq(&bound.0, &again.0));
        assert_eq!(budget.snapshot().retained_bytes - before, 2 * fixed);
        assert!(bound.bind(&names, second).is_err());
        assert!(bound.check_head(&names, Some(second)).is_err());
        assert!(retained.check_head(&names, Some(first)).is_err());
        drop(retained);
        assert_eq!(budget.snapshot().retained_bytes - before, fixed);
        drop(bound);
        assert_eq!(budget.snapshot().retained_bytes - before, fixed);
        drop(again);
        assert_eq!(budget.snapshot().retained_bytes, before);
        drop(names);
        assert_eq!(budget.snapshot().retained_bytes, 0);
    }

    #[test]
    fn retained_proof_denies_storage_before_arc_allocation_and_latches() {
        let healthy = ExpansionBudget::default();
        let (names, _, _) = fixture(healthy.clone());
        let baseline = healthy.snapshot().retained_bytes;
        drop(names);
        let fixed = (size_of::<RetainedAnchorData>() + 2 * size_of::<usize>()) as u64;
        let budget = ExpansionBudget::new(ExpansionLimits {
            bytes: baseline + fixed - 1,
            ..ExpansionLimits::default()
        });
        let (names, proof, _) = fixture(budget.clone());
        let Err(TokenError::Budget(first)) = proof.retain(names.session()) else {
            panic!("proof allocation must be precharged");
        };
        assert_eq!(budget.snapshot().retained_bytes, baseline);
        let Err(TokenError::Budget(again)) = proof.retain(names.session()) else {
            panic!("proof allocation failure must latch");
        };
        assert_eq!(first, again);
        assert_eq!(budget.snapshot().retained_bytes, baseline);
        drop(names);
        assert_eq!(budget.snapshot().retained_bytes, 0);
    }
}
