//! Owned identity prerequisite for retaining typed frontend values, not an artifact cache.
//!
//! All byte identities below must contain canonical, exact identity material, not just a
//! fingerprint. In particular, `source` must identify exact source contents (including
//! relevant external inputs), and `options` all semantic options. Callers own completeness
//! of that material. Equality here is a necessary condition, never permission to extend a
//! compiler lifetime or move values between context domains. The context domain must also
//! be retained and validated separately.

use std::collections::BTreeSet;

/// Exact stable crate identity and its session-local numeric assignment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CrateIdentity {
    pub stable: Vec<u8>,
    pub crate_index: u32,
}

/// Exact stable definition identity and its session-local numeric assignments.
///
/// Entries must include every definition reachable by retained typed values, including
/// external definitions. Preserve allocation order. An absent owner is distinct from 0.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DefinitionIdentity {
    pub stable: Vec<u8>,
    pub crate_index: u32,
    pub def_index: u32,
    pub owner_index: Option<u32>,
}

/// An owned snapshot. No compiler pointers, arena references, or hash-only keys.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RevisionIdentity {
    pub namespace: Vec<u8>,
    pub options: Vec<u8>,
    /// The complete ordered crate universe, not only directly referenced crates.
    pub crates: Vec<CrateIdentity>,
    pub definitions: Vec<DefinitionIdentity>,
    pub source: Vec<u8>,
    /// Exact revision token for body/semantic inputs not represented in `source`.
    pub body_revision: Vec<u8>,
}

/// Per-snapshot bounds. Exceeding any bound invalidates the whole resident domain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IdentityBudget {
    pub max_crates: usize,
    pub max_definitions: usize,
    pub max_identity_bytes: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResetReason {
    CrateBudget,
    DefinitionBudget,
    IdentityByteBudget,
    NamespaceChanged,
    OptionsChanged,
    CrateUniverseChanged,
    DefinitionMappingChanged,
    DuplicateCrateIdentity,
    DuplicateCrateIndex,
    DuplicateDefinitionIdentity,
    DuplicateDefinitionIndex,
    DuplicateOwnerIndex,
    DanglingDefinitionCrate,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RevisionGate {
    SourceChanged,
    BodyRevisionChanged,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compatibility {
    /// Identity prerequisite passed. The owning context domain must still authorize use.
    Reusable,
    /// Numeric/stable identities match, but old typed results must not be reused.
    IdentityOnly(RevisionGate),
    /// Discard the resident domain, including any retained typed values.
    Reset(ResetReason),
}

/// Opaque evidence that these two exact immutable snapshots passed the identity gate.
///
/// This is not an owner-green proof and does not authorize transferring values between
/// compiler context domains. A consumer must independently bind stored values to its
/// retained context and to `previous`. Changed source/body inputs never yield this token.
/// Borrowing snapshots prevents their mutation while the authorization is alive.
#[derive(Debug)]
pub struct ReuseAuthorization<'a> {
    previous: &'a RevisionIdentity,
    next: &'a RevisionIdentity,
    budget: IdentityBudget,
}

impl ReuseAuthorization<'_> {
    /// Bind authorization to the actual checked snapshot objects, not merely equal
    /// caller-provided data or a freely constructible `Compatibility::Reusable` value.
    pub fn authorizes(&self, previous: &RevisionIdentity, next: &RevisionIdentity) -> bool {
        std::ptr::eq(self.previous, previous) && std::ptr::eq(self.next, next)
    }

    pub fn budget(&self) -> IdentityBudget {
        self.budget
    }
}

impl RevisionIdentity {
    /// Issue non-forgeable evidence only for complete, well-formed, unchanged revisions.
    /// Identity-only matches are deliberately errors until a separate owner-green path
    /// can validate individual retained results inside their original context domain.
    pub fn authorize_reuse<'a>(
        &'a self,
        next: &'a Self,
        budget: IdentityBudget,
    ) -> Result<ReuseAuthorization<'a>, Compatibility> {
        match self.compatibility(next, budget) {
            Compatibility::Reusable => Ok(ReuseAuthorization {
                previous: self,
                next,
                budget,
            }),
            incompatible => Err(incompatible),
        }
    }

    /// Check both snapshots before authorizing any reuse. Comparisons are exact and
    /// ordered: matching old numeric IDs or a matching fingerprint cannot pass alone.
    /// When several inputs differ, return the first reason in domain-to-revision order.
    pub fn compatibility(&self, next: &Self, budget: IdentityBudget) -> Compatibility {
        if let Err(reason) = self
            .check_budget(budget)
            .and_then(|()| next.check_budget(budget))
            .and_then(|()| self.check_mapping())
            .and_then(|()| next.check_mapping())
        {
            return Compatibility::Reset(reason);
        }
        let reset = if self.namespace != next.namespace {
            Some(ResetReason::NamespaceChanged)
        } else if self.options != next.options {
            Some(ResetReason::OptionsChanged)
        } else if self.crates != next.crates {
            Some(ResetReason::CrateUniverseChanged)
        } else if self.definitions != next.definitions {
            Some(ResetReason::DefinitionMappingChanged)
        } else {
            None
        };
        if let Some(reason) = reset {
            Compatibility::Reset(reason)
        } else if self.source != next.source {
            Compatibility::IdentityOnly(RevisionGate::SourceChanged)
        } else if self.body_revision != next.body_revision {
            Compatibility::IdentityOnly(RevisionGate::BodyRevisionChanged)
        } else {
            Compatibility::Reusable
        }
    }

    /// Validate exact identities and numeric assignments, including equal malformed snapshots.
    /// Budget checking must precede this allocation-bounded structural validation.
    fn check_mapping(&self) -> Result<(), ResetReason> {
        let mut crate_ids = BTreeSet::new();
        let mut crate_indices = BTreeSet::new();
        for krate in &self.crates {
            if !crate_ids.insert(krate.stable.as_slice()) {
                return Err(ResetReason::DuplicateCrateIdentity);
            }
            if !crate_indices.insert(krate.crate_index) {
                return Err(ResetReason::DuplicateCrateIndex);
            }
        }
        let mut def_ids = BTreeSet::new();
        let mut def_indices = BTreeSet::new();
        let mut owner_indices = BTreeSet::new();
        for def in &self.definitions {
            if !crate_indices.contains(&def.crate_index) {
                return Err(ResetReason::DanglingDefinitionCrate);
            }
            if !def_ids.insert((def.crate_index, def.stable.as_slice())) {
                return Err(ResetReason::DuplicateDefinitionIdentity);
            }
            if !def_indices.insert((def.crate_index, def.def_index)) {
                return Err(ResetReason::DuplicateDefinitionIndex);
            }
            if let Some(owner) = def.owner_index {
                if !owner_indices.insert((def.crate_index, owner)) {
                    return Err(ResetReason::DuplicateOwnerIndex);
                }
            }
        }
        Ok(())
    }

    /// Allows a caller to reject an oversized snapshot before retaining it.
    pub fn check_budget(&self, budget: IdentityBudget) -> Result<(), ResetReason> {
        if self.crates.len() > budget.max_crates {
            return Err(ResetReason::CrateBudget);
        }
        if self.definitions.len() > budget.max_definitions {
            return Err(ResetReason::DefinitionBudget);
        }
        let lengths = [
            self.namespace.len(),
            self.options.len(),
            self.source.len(),
            self.body_revision.len(),
        ];
        let mut remaining = budget.max_identity_bytes;
        for len in lengths
            .into_iter()
            .chain(self.crates.iter().map(|krate| krate.stable.len()))
            .chain(self.definitions.iter().map(|def| def.stable.len()))
        {
            remaining = remaining
                .checked_sub(len)
                .ok_or(ResetReason::IdentityByteBudget)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BUDGET: IdentityBudget = IdentityBudget {
        max_crates: 4,
        max_definitions: 8,
        max_identity_bytes: 1024,
    };

    fn snapshot() -> RevisionIdentity {
        RevisionIdentity {
            namespace: b"domain".to_vec(),
            options: b"semantic options".to_vec(),
            crates: vec![CrateIdentity {
                stable: b"local crate".to_vec(),
                crate_index: 0,
            }],
            definitions: vec![
                DefinitionIdentity {
                    stable: b"crate::a".to_vec(),
                    crate_index: 0,
                    def_index: 1,
                    owner_index: Some(1),
                },
                DefinitionIdentity {
                    stable: b"crate::b".to_vec(),
                    crate_index: 0,
                    def_index: 2,
                    owner_index: Some(2),
                },
            ],
            source: b"fn a() {} fn b() {}".to_vec(),
            body_revision: b"revision 1".to_vec(),
        }
    }

    #[test]
    fn exact_owned_snapshot_matches() {
        let old = snapshot();
        assert_eq!(
            old.compatibility(&old.clone(), BUDGET),
            Compatibility::Reusable
        );
    }

    #[test]
    fn reordered_inserted_removed_definitions_reset() {
        let old = snapshot();
        let mut reordered = old.clone();
        reordered.definitions.swap(0, 1);
        let mut inserted = old.clone();
        inserted.definitions.push(DefinitionIdentity {
            stable: b"crate::c".to_vec(),
            crate_index: 0,
            def_index: 3,
            owner_index: Some(3),
        });
        let mut removed = old.clone();
        removed.definitions.pop();
        for next in [reordered, inserted, removed] {
            assert_eq!(
                old.compatibility(&next, BUDGET),
                Compatibility::Reset(ResetReason::DefinitionMappingChanged)
            );
        }
    }

    #[test]
    fn neither_old_numbers_nor_stable_identity_alone_authorizes() {
        let mut old = snapshot();
        old.crates.push(CrateIdentity {
            stable: b"dep".to_vec(),
            crate_index: 1,
        });
        for field in 0..4 {
            let mut next = old.clone();
            let def = &mut next.definitions[0];
            match field {
                0 => def.stable = b"crate::different".to_vec(),
                1 => def.def_index += 10,
                2 => def.owner_index = None,
                _ => def.crate_index += 1,
            }
            assert_eq!(
                old.compatibility(&next, BUDGET),
                Compatibility::Reset(ResetReason::DefinitionMappingChanged)
            );
        }
    }

    #[test]
    fn source_and_body_revisions_gate_typed_values_not_identity() {
        let old = snapshot();
        let mut next = old.clone();
        next.source.push(b' ');
        assert_eq!(
            old.compatibility(&next, BUDGET),
            Compatibility::IdentityOnly(RevisionGate::SourceChanged)
        );
        next = old.clone();
        next.body_revision.push(b'2');
        assert_eq!(
            old.compatibility(&next, BUDGET),
            Compatibility::IdentityOnly(RevisionGate::BodyRevisionChanged)
        );
    }

    #[test]
    fn domain_and_crate_universe_changes_reset() {
        let old = snapshot();
        for field in 0..6 {
            let mut next = old.clone();
            let reason = match field {
                0 => {
                    next.namespace.push(0);
                    ResetReason::NamespaceChanged
                }
                1 => {
                    next.options.push(0);
                    ResetReason::OptionsChanged
                }
                2 => {
                    next.crates[0].stable.push(0);
                    ResetReason::CrateUniverseChanged
                }
                3 => {
                    next.crates[0].crate_index = 1;
                    for def in &mut next.definitions {
                        def.crate_index = 1;
                    }
                    ResetReason::CrateUniverseChanged
                }
                4 => {
                    next.crates.push(CrateIdentity {
                        stable: b"dep".to_vec(),
                        crate_index: 1,
                    });
                    ResetReason::CrateUniverseChanged
                }
                _ => {
                    next.crates.clear();
                    next.definitions.clear();
                    ResetReason::CrateUniverseChanged
                }
            };
            assert_eq!(
                old.compatibility(&next, BUDGET),
                Compatibility::Reset(reason)
            );
        }
    }

    #[test]
    fn crate_order_and_exact_budget_boundary() {
        let mut old = snapshot();
        old.crates.push(CrateIdentity {
            stable: b"dependency".to_vec(),
            crate_index: 1,
        });
        let mut next = old.clone();
        next.crates.swap(0, 1);
        assert_eq!(
            old.compatibility(&next, BUDGET),
            Compatibility::Reset(ResetReason::CrateUniverseChanged)
        );
        let bytes = old.namespace.len()
            + old.options.len()
            + old.source.len()
            + old.body_revision.len()
            + old
                .crates
                .iter()
                .map(|entry| entry.stable.len())
                .sum::<usize>()
            + old
                .definitions
                .iter()
                .map(|entry| entry.stable.len())
                .sum::<usize>();
        let budget = IdentityBudget {
            max_crates: old.crates.len(),
            max_definitions: old.definitions.len(),
            max_identity_bytes: bytes,
        };
        assert_eq!(old.compatibility(&old, budget), Compatibility::Reusable);
        assert_eq!(
            old.check_budget(IdentityBudget {
                max_identity_bytes: bytes - 1,
                ..budget
            }),
            Err(ResetReason::IdentityByteBudget)
        );
    }

    #[test]
    fn authorization_is_bound_to_actual_snapshots_and_budget() {
        let old = snapshot();
        let next = old.clone();
        let authorization = old.authorize_reuse(&next, BUDGET).unwrap();
        assert!(authorization.authorizes(&old, &next));
        assert_eq!(authorization.budget(), BUDGET);
        assert!(!authorization.authorizes(&old.clone(), &next));
        assert!(!authorization.authorizes(&old, &next.clone()));
        assert!(!authorization.authorizes(&next, &old));
    }

    #[test]
    fn source_body_and_budget_failures_cannot_issue_authorization() {
        let old = snapshot();
        let mut next = old.clone();
        next.source.push(0);
        assert!(matches!(
            old.authorize_reuse(&next, BUDGET),
            Err(Compatibility::IdentityOnly(RevisionGate::SourceChanged))
        ));
        next = old.clone();
        next.body_revision.push(0);
        assert!(matches!(
            old.authorize_reuse(&next, BUDGET),
            Err(Compatibility::IdentityOnly(
                RevisionGate::BodyRevisionChanged
            ))
        ));
        assert!(matches!(
            old.authorize_reuse(
                &old,
                IdentityBudget {
                    max_crates: 0,
                    ..BUDGET
                }
            ),
            Err(Compatibility::Reset(ResetReason::CrateBudget))
        ));
    }

    #[test]
    fn equal_malformed_mappings_fail_closed() {
        let valid = snapshot();
        for field in 0..6 {
            let mut invalid = valid.clone();
            let reason = match field {
                0 => {
                    invalid.crates.push(CrateIdentity {
                        stable: invalid.crates[0].stable.clone(),
                        crate_index: 1,
                    });
                    ResetReason::DuplicateCrateIdentity
                }
                1 => {
                    invalid.crates.push(CrateIdentity {
                        stable: b"other crate".to_vec(),
                        crate_index: 0,
                    });
                    ResetReason::DuplicateCrateIndex
                }
                2 => {
                    invalid.definitions[1].stable = invalid.definitions[0].stable.clone();
                    ResetReason::DuplicateDefinitionIdentity
                }
                3 => {
                    invalid.definitions[1].def_index = invalid.definitions[0].def_index;
                    ResetReason::DuplicateDefinitionIndex
                }
                4 => {
                    invalid.definitions[1].owner_index = invalid.definitions[0].owner_index;
                    ResetReason::DuplicateOwnerIndex
                }
                _ => {
                    invalid.definitions[0].crate_index = 99;
                    ResetReason::DanglingDefinitionCrate
                }
            };
            assert!(invalid.authorize_reuse(&invalid, BUDGET).is_err());
            assert_eq!(
                invalid.compatibility(&invalid, BUDGET),
                Compatibility::Reset(reason)
            );
            assert_eq!(
                valid.compatibility(&invalid, BUDGET),
                Compatibility::Reset(reason)
            );
            assert_eq!(
                invalid.compatibility(&valid, BUDGET),
                Compatibility::Reset(reason)
            );
        }
    }

    #[test]
    fn definition_and_owner_numbers_are_scoped_to_crate() {
        let mut valid = snapshot();
        valid.crates.push(CrateIdentity {
            stable: b"dep".to_vec(),
            crate_index: 1,
        });
        let mut external = valid.definitions[0].clone();
        external.crate_index = 1;
        valid.definitions.push(external);
        valid.definitions[0].owner_index = None;
        valid.definitions[1].owner_index = None;
        assert_eq!(valid.compatibility(&valid, BUDGET), Compatibility::Reusable);
    }

    #[test]
    fn budget_failure_resets_even_equal_snapshots() {
        let old = snapshot();
        for (budget, reason) in [
            (
                IdentityBudget {
                    max_crates: 0,
                    ..BUDGET
                },
                ResetReason::CrateBudget,
            ),
            (
                IdentityBudget {
                    max_definitions: 1,
                    ..BUDGET
                },
                ResetReason::DefinitionBudget,
            ),
            (
                IdentityBudget {
                    max_identity_bytes: 1,
                    ..BUDGET
                },
                ResetReason::IdentityByteBudget,
            ),
        ] {
            assert_eq!(
                old.compatibility(&old, budget),
                Compatibility::Reset(reason)
            );
        }
        let mut next = old.clone();
        next.source.resize(BUDGET.max_identity_bytes + 1, 0);
        assert_eq!(
            old.compatibility(&next, BUDGET),
            Compatibility::Reset(ResetReason::IdentityByteBudget)
        );
        assert_eq!(
            next.compatibility(&old, BUDGET),
            Compatibility::Reset(ResetReason::IdentityByteBudget)
        );
    }
}
