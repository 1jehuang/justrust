//! Typed domain flavors reuse per-owner kind 115, never a new raw node kind.
use super::{ItemKey, Key};
use crate::compiler::ir::{DeclId, SnapshotId};

#[derive(Clone)]
pub(in crate::compiler) enum AlignmentDomain {
    Header(DeclId),
    Topology(DeclId),
    Summary(Vec<DeclId>),
}
impl ItemKey for AlignmentDomain {
    const KIND: u32 = crate::compiler::UNIVERSAL_EFFECTS_READY;
    fn params(&self) -> (u64, Vec<u64>) {
        let (flavor, owners): (u64, &[DeclId]) = match self {
            Self::Header(owner) => (0, std::slice::from_ref(owner)),
            Self::Topology(owner) => (1, std::slice::from_ref(owner)),
            Self::Summary(owners) => (2, owners),
        };
        let mut words = Vec::with_capacity(1 + 2 * owners.len());
        words.push(flavor);
        for owner in owners {
            words.extend([owner.snapshot.graph_word(), owner.index]);
        }
        (u64::MAX - 1, words)
    }
}
impl AlignmentDomain {
    pub(in crate::compiler) fn charge_key(
        &self,
        budget: &crate::compiler::ir::PlanBudget,
    ) -> Result<(), String> {
        let owners = match self {
            Self::Summary(owners) => owners.len(),
            _ => 1,
        };
        let words = owners
            .checked_mul(2)
            .and_then(|n| n.checked_add(2))
            .ok_or("alignment key extent overflow")?;
        budget.charge_work(words)?;
        budget.retain_bytes(
            words
                .checked_mul(2 * std::mem::size_of::<u64>())
                .ok_or("alignment key storage overflow")?,
        )
    }
    pub(in crate::compiler) fn decode(key: &Key) -> Result<Option<Self>, String> {
        if key.kind != Self::KIND || key.params.first() != Some(&(u64::MAX - 1)) {
            return Ok(None);
        }
        let Some((&flavor, rest)) = key.params.get(1..).and_then(|words| words.split_first())
        else {
            return Err("alignment domain key omitted its typed flavor".into());
        };
        if rest.is_empty() || rest.len() % 2 != 0 {
            return Err("alignment domain key has malformed owner identities".into());
        }
        let owners: Vec<_> = rest
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| DeclId {
                snapshot: SnapshotId::from_graph_word(pair[0]),
                index: pair[1],
            })
            .collect();
        Ok(Some(match (flavor, owners.as_slice()) {
            (0, [owner]) => Self::Header(*owner),
            (1, [owner]) => Self::Topology(*owner),
            (2, _) if owners.windows(2).all(|pair| pair[0] < pair[1]) => Self::Summary(owners),
            _ => {
                return Err(
                    "alignment domain key changed canonical flavor or SCC member order".into(),
                );
            }
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn alignment_domain_key_rejects_noncanonical_members_and_flavors() {
        let owner = DeclId {
            snapshot: SnapshotId::from_graph_word(7),
            index: 3,
        };
        let denied = crate::compiler::ir::PlanBudget::new(crate::compiler::ir::PlanLimits {
            bytes: 0,
            ..Default::default()
        });
        assert!(AlignmentDomain::Header(owner).charge_key(&denied).is_err());
        let authentic = AlignmentDomain::Header(owner).key();
        assert!(
            matches!(AlignmentDomain::decode(&authentic).unwrap(), Some(AlignmentDomain::Header(value)) if value == owner)
        );
        let mut forged = authentic.clone();
        forged.params[1] = 9;
        assert!(AlignmentDomain::decode(&forged).is_err());
        let duplicate = AlignmentDomain::Summary(vec![owner, owner]).key();
        assert!(AlignmentDomain::decode(&duplicate).is_err());
        let empty = AlignmentDomain::Summary(vec![]).key();
        assert!(AlignmentDomain::decode(&empty).is_err());
        let mut truncated = authentic;
        truncated.params.pop();
        assert!(AlignmentDomain::decode(&truncated).is_err());
    }
}
