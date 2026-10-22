//! Immutable crate view over per-file name stores (PARSE-SPLIT.md step 2b).
//!
//! The view is an O(files) index from session owner to the file's frozen
//! `Arc<NameStore>`. It never copies names. A handle whose owner is not one
//! of the crate's files is a hard `ForeignHandle`, never a fallback.
//! Name keys from two files meet only through `same_key`, which verifies
//! spelling behind the fingerprint fast path.
use std::sync::Arc;

use super::super::token_session::{SessionId, TokenError};
use super::{AstIdent, LookupClass, NameData, NameError, NameKey, NameStore};

/// A name key tagged with the file store that produced it, so a cross-file
/// comparison can always verify spelling against the right owner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CrateNameKey {
    owner: SessionId,
    key: NameKey,
}
impl CrateNameKey {
    pub fn key(self) -> NameKey {
        self.key
    }
}

#[derive(Debug)]
pub struct CrateNames {
    /// Sorted by owner, unique. Explicit membership: exactly the crate's files.
    files: Box<[(SessionId, Arc<NameStore>)]>,
}

impl CrateNames {
    /// Assemble from the crate's frozen file stores. Duplicate owners (the
    /// same store twice, or two stores claiming one owner) are rejected.
    pub fn new(stores: Vec<Arc<NameStore>>) -> Result<Self, NameError> {
        let mut files: Vec<(SessionId, Arc<NameStore>)> = Vec::with_capacity(stores.len());
        for store in stores {
            store
                .budget()
                .charge(super::super::expansion_budget::Counter::Work, 1)?;
            files.push((store.session().id(), store));
        }
        files.sort_unstable_by_key(|(owner, _)| owner.bits());
        if files.windows(2).any(|pair| pair[0].0 == pair[1].0) {
            return Err(TokenError::InvalidInput("duplicate file owner in crate view").into());
        }
        Ok(Self {
            files: files.into_boxed_slice(),
        })
    }
    pub fn files(&self) -> usize {
        self.files.len()
    }
    /// The file store owning `owner`, or a hard foreign-handle error.
    fn store(&self, owner: SessionId) -> Result<&NameStore, NameError> {
        self.files
            .binary_search_by_key(&owner.bits(), |(id, _)| id.bits())
            .map(|index| &*self.files[index].1)
            .map_err(|_| TokenError::ForeignHandle.into())
    }
    pub fn store_of(&self, ident: AstIdent) -> Result<&NameStore, NameError> {
        self.store(ident.owner)
    }
    pub fn check(&self, ident: AstIdent) -> Result<(), NameError> {
        self.store_of(ident)?.check(ident)
    }
    pub fn spelling(&self, ident: AstIdent) -> Result<&str, NameError> {
        self.store_of(ident)?.spelling(ident)
    }
    pub fn data(&self, ident: AstIdent) -> Result<NameData, NameError> {
        self.store_of(ident)?.data(ident)
    }
    pub fn key(&self, ident: AstIdent, class: LookupClass) -> Result<CrateNameKey, NameError> {
        Ok(CrateNameKey {
            owner: ident.owner,
            key: self.store_of(ident)?.key(ident, class)?,
        })
    }
    /// Verified semantic equality of keys that may come from different files.
    pub fn same_key(&self, a: CrateNameKey, b: CrateNameKey) -> Result<bool, NameError> {
        let left = self.store(a.owner)?;
        let right = self.store(b.owner)?;
        left.same_key_across(a.key, right, b.key)
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::expansion_budget::ExpansionBudget;
    use super::super::super::token::Edition;
    use super::super::super::token_session::{AtomFingerprint, SourceSnapshot, TokenSession};
    use super::super::NameStoreBuilder;
    use super::*;
    use std::path::Path;

    /// One file store holding root-context occurrences of `names`, spelled
    /// honestly unless `forge` maps a spelling onto another's fingerprint.
    fn file(
        path: &str,
        names: &[&str],
        forge: Option<(&str, &str)>,
    ) -> (
        Arc<NameStore>,
        Vec<AstIdent>,
        Option<super::super::super::token_session::SymbolId>,
    ) {
        let budget = ExpansionBudget::default();
        let text = format!("{}\n", names.join(" "));
        let source = SourceSnapshot::from_text(
            Path::new(path),
            &text,
            &budget,
            crate::compiler::owner_path::standalone_issued(
                &budget,
                crate::compiler::owner_path::OwnerDomain::SourceSnapshot,
            )
            .unwrap(),
        )
        .unwrap();
        let handle = source.file(0).unwrap();
        let origin = source.span(handle, 0, 0).unwrap();
        let (mut session, cap) = TokenSession::standalone(source, budget).unwrap();
        let origin = session.original_span(origin).unwrap();
        let forged = forge.map(|(spelling, onto)| {
            session
                .intern_forged(spelling, AtomFingerprint::of(onto))
                .unwrap()
        });
        let mut builder = NameStoreBuilder::new(session, cap, Edition::E2024).unwrap();
        let idents = names
            .iter()
            .map(|name| builder.intern_root(name, origin).unwrap())
            .collect();
        (builder.freeze().unwrap(), idents, forged)
    }

    #[test]
    fn a_uses_item_declared_in_b_through_the_crate_view() {
        // b.rs declares `helper`; a.rs refers to `helper` and `other`.
        let (b, b_names, _) = file("b.rs", &["helper"], None);
        let (a, a_names, _) = file("a.rs", &["helper", "other"], None);
        let view = CrateNames::new(vec![a.clone(), b.clone()]).unwrap();
        assert_eq!(view.files(), 2);
        let class = LookupClass::Item(super::super::Namespace::Value);
        let decl = view.key(b_names[0], class).unwrap();
        let use_site = view.key(a_names[0], class).unwrap();
        let other = view.key(a_names[1], class).unwrap();
        assert!(view.same_key(use_site, decl).unwrap());
        assert!(!view.same_key(other, decl).unwrap());
        assert_eq!(view.spelling(a_names[0]).unwrap(), "helper");
        assert_eq!(view.spelling(b_names[0]).unwrap(), "helper");
        // The view indexes Arcs; it does not copy names.
        assert_eq!(Arc::strong_count(&a), 2);
    }

    #[test]
    fn colliding_spelling_across_files_is_rejected_not_resolved() {
        let (b, b_names, _) = file("b.rs", &["helper"], None);
        // a.rs's distinct `evil` is forged onto `helper`'s fingerprint.
        let (a, a_names, forged) = file("a.rs", &["evil"], Some(("evil", "helper")));
        let view = CrateNames::new(vec![a.clone(), b]).unwrap();
        let class = LookupClass::Item(super::super::Namespace::Value);
        let decl = view.key(b_names[0], class).unwrap();
        let honest = view.key(a_names[0], class).unwrap();
        assert!(!view.same_key(honest, decl).unwrap());
        let forged = CrateNameKey {
            owner: a_names[0].owner,
            key: NameKey {
                atom: super::super::NameAtomId::new(a.session(), forged.unwrap()).unwrap(),
                ..decl.key
            },
        };
        assert_eq!(
            forged.key(),
            decl.key(),
            "fingerprints collide by construction"
        );
        assert!(matches!(
            view.same_key(forged, decl),
            Err(NameError::Token(TokenError::AtomCollision))
        ));
    }

    #[test]
    fn linked_sibling_store_serves_its_handles_and_rejects_outsiders() {
        let (b, b_names, _) = file("b.rs", &["helper"], None);
        let (outside, outside_names, _) = file("c.rs", &["x"], None);
        let budget = ExpansionBudget::default();
        let source = SourceSnapshot::from_text(
            Path::new("a.rs"),
            "a\n",
            &budget,
            crate::compiler::owner_path::standalone_issued(
                &budget,
                crate::compiler::owner_path::OwnerDomain::SourceSnapshot,
            )
            .unwrap(),
        )
        .unwrap();
        let (session, cap) = TokenSession::standalone(source, budget).unwrap();
        let mut builder = NameStoreBuilder::new(session, cap, Edition::E2024).unwrap();
        builder.link_crate_files(vec![b.clone()]).unwrap();
        assert!(builder.link_crate_files(vec![b.clone()]).is_err());
        assert_eq!(builder.spelling(b_names[0]).unwrap(), "helper");
        let span = b.span(b_names[0]).unwrap();
        assert!(builder.session().span(span).is_ok());
        assert!(matches!(
            builder.check(outside_names[0]),
            Err(NameError::Token(TokenError::ForeignHandle))
        ));
        let key = b
            .key(
                b_names[0],
                LookupClass::Item(super::super::Namespace::Value),
            )
            .unwrap();
        assert_eq!(builder.atom_text(key.atom).unwrap(), "helper");
        drop(outside);
    }

    #[test]
    fn handles_from_outside_the_crate_are_hard_foreign_errors() {
        let (a, _, _) = file("a.rs", &["x"], None);
        let (outside, outside_names, _) = file("c.rs", &["x"], None);
        let view = CrateNames::new(vec![a.clone()]).unwrap();
        assert!(matches!(
            view.check(outside_names[0]),
            Err(NameError::Token(TokenError::ForeignHandle))
        ));
        let class = LookupClass::Item(super::super::Namespace::Value);
        assert!(matches!(
            view.key(outside_names[0], class),
            Err(NameError::Token(TokenError::ForeignHandle))
        ));
        drop(outside);
        assert!(CrateNames::new(vec![a.clone(), a]).is_err());
    }
}
