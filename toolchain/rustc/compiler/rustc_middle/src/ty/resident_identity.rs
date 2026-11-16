//! Owned identity prerequisite for retaining typed frontend values, not an artifact cache.
//!
//! All byte identities below must contain canonical, exact identity material, not just a
//! fingerprint. In particular, `source` must identify exact source contents (including
//! relevant external inputs), and `options` all semantic options. Callers own completeness
//! of that material. Equality here is a necessary condition, never permission to extend a
//! compiler lifetime or move values between context domains. The context domain must also
//! be retained and validated separately.

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

impl RevisionIdentity {
    /// Check both snapshots before authorizing any reuse. Comparisons are exact and
    /// ordered: matching old numeric IDs or a matching fingerprint cannot pass alone.
    /// When several inputs differ, return the first reason in domain-to-revision order.
    pub fn compatibility(&self, next: &Self, budget: IdentityBudget) -> Compatibility {
        if let Err(reason) = self
            .check_budget(budget)
            .and_then(|()| next.check_budget(budget))
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
        let old = snapshot();
        for field in 0..4 {
            let mut next = old.clone();
            let def = &mut next.definitions[0];
            match field {
                0 => def.stable = b"crate::different".to_vec(),
                1 => def.def_index += 1,
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
