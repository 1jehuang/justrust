//! Canonical closed-owner/SCC flavors of source admission, kind 128.
use super::{ItemKey, Key};
use crate::compiler::{instances, ir};
#[derive(Clone)]
pub(in crate::compiler) enum ClosedDomain {
    Header(Vec<u64>),
    Topology(Vec<u64>),
    Summary(Vec<Vec<u64>>),
}
impl ClosedDomain {
    pub(in crate::compiler) fn charge_key(&self, budget: &ir::PlanBudget) -> Result<(), String> {
        let members: &[Vec<u64>] = match self {
            Self::Header(words) | Self::Topology(words) => std::slice::from_ref(words),
            Self::Summary(members) => members,
        };
        let mut words = 2usize;
        for member in members {
            budget.charge_work(1)?;
            words = words
                .checked_add(1)
                .and_then(|n| n.checked_add(member.len()))
                .ok_or("closed alignment key extent overflow")?;
        }
        budget.charge_work(words)?;
        budget.retain_bytes(
            words
                .checked_mul(2 * std::mem::size_of::<u64>())
                .ok_or("closed alignment key storage overflow")?,
        )
    }
    #[cfg(test)]
    pub(in crate::compiler) fn header(
        target: &ir::CallTarget,
        budget: &ir::PlanBudget,
    ) -> Result<Self, String> {
        Ok(Self::Header(instances::graph_key::encode(target, budget)?))
    }
    pub(in crate::compiler) fn topology(
        target: &ir::CallTarget,
        budget: &ir::PlanBudget,
    ) -> Result<Self, String> {
        Ok(Self::Topology(instances::graph_key::encode(
            target, budget,
        )?))
    }
    pub(in crate::compiler) fn decode(key: &Key) -> Result<Option<Self>, String> {
        if key.kind != Self::KIND || key.params.first() != Some(&(u64::MAX - 2)) {
            return Ok(None);
        }
        let Some((&flavor, mut words)) = key.params.get(1..).and_then(|words| words.split_first())
        else {
            return Err("closed alignment key omitted flavor".into());
        };
        let mut members = Vec::new();
        while let Some((&length, rest)) = words.split_first() {
            let length =
                usize::try_from(length).map_err(|_| "closed alignment key length overflow")?;
            if length == 0 || length > rest.len() {
                return Err("closed alignment key has invalid member extent".into());
            }
            members.push(rest[..length].to_vec());
            words = &rest[length..];
        }
        Ok(Some(match (flavor, members.as_slice()) {
            (0, [member]) => Self::Header(member.clone()),
            (1, [member]) => Self::Topology(member.clone()),
            (2, _) if !members.is_empty() && members.windows(2).all(|pair| pair[0] < pair[1]) => {
                Self::Summary(members)
            }
            _ => return Err("closed alignment key changed canonical flavor/member order".into()),
        }))
    }
}
impl ItemKey for ClosedDomain {
    const KIND: u32 = crate::compiler::SOURCE_ADMISSION;
    fn params(&self) -> (u64, Vec<u64>) {
        let (flavor, members): (u64, &[Vec<u64>]) = match self {
            Self::Header(words) => (0, std::slice::from_ref(words)),
            Self::Topology(words) => (1, std::slice::from_ref(words)),
            Self::Summary(members) => (2, members),
        };
        let mut words = vec![flavor];
        for member in members {
            words.push(member.len() as u64);
            words.extend_from_slice(member);
        }
        (u64::MAX - 2, words)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn closed_alignment_key_preserves_real_owner_and_ordered_actuals() {
        let budget = ir::PlanBudget::default();
        let target = ir::CallTarget {
            declaration: ir::DeclId {
                snapshot: ir::SnapshotId::standalone().unwrap(),
                index: 42,
            },
            type_args: std::sync::Arc::from([ir::Ty::Int(ir::IntTy::U16)]),
        };
        let first = ClosedDomain::header(&target, &budget).unwrap().key();
        assert_eq!(first, ClosedDomain::header(&target, &budget).unwrap().key());
        assert_ne!(
            first,
            ClosedDomain::topology(&target, &budget).unwrap().key()
        );
        let mut changed = target;
        changed.type_args = std::sync::Arc::from([ir::Ty::Int(ir::IntTy::U32)]);
        assert_ne!(
            first,
            ClosedDomain::header(&changed, &budget).unwrap().key()
        );
        changed.declaration.index += 1;
        assert_ne!(
            first,
            ClosedDomain::header(&changed, &budget).unwrap().key()
        );
        let denied = ir::PlanBudget::new(ir::PlanLimits {
            work: 0,
            ..Default::default()
        });
        assert!(ClosedDomain::header(&changed, &denied).is_err());
    }
    #[test]
    fn closed_alignment_key_rejects_extent_flavor_and_scc_forgery() {
        let denied = ir::PlanBudget::new(ir::PlanLimits {
            bytes: 0,
            ..Default::default()
        });
        assert!(
            ClosedDomain::Header(vec![3, 7, 11])
                .charge_key(&denied)
                .is_err()
        );
        let authentic = ClosedDomain::Header(vec![3, 7, 11]).key();
        assert!(
            matches!(ClosedDomain::decode(&authentic).unwrap(), Some(ClosedDomain::Header(words)) if words == [3, 7, 11])
        );
        let mut bad = authentic.clone();
        bad.params[2] = 99;
        assert!(ClosedDomain::decode(&bad).is_err());
        bad = authentic.clone();
        bad.params[1] = 4;
        assert!(ClosedDomain::decode(&bad).is_err());
        for members in [vec![], vec![vec![2], vec![1]], vec![vec![1], vec![1]]] {
            assert!(ClosedDomain::decode(&ClosedDomain::Summary(members).key()).is_err());
        }
        let mut truncated = authentic;
        truncated.params.pop();
        assert!(ClosedDomain::decode(&truncated).is_err());
    }
}
