//! Source-owned opaque declaration flavors of source admission, kind 128.
//! Headers cannot wait on validation, so recursive declaration edges never
//! become scheduler cycles. The tag is disjoint from ordinary source owners
//! and the existing closed-alignment flavor.
use super::{ItemKey, Key};
use crate::compiler::ir;

pub(in crate::compiler) const TAG: u64 = u64::MAX - 3;

#[derive(Clone, Copy, Debug)]
pub(in crate::compiler) enum OpaqueDeclaration {
    Header(ir::DeclId),
    Validate(ir::DeclId),
}

impl OpaqueDeclaration {
    pub(in crate::compiler) fn charge_key(&self, budget: &ir::PlanBudget) -> Result<(), String> {
        budget.charge_work(4)?;
        // The typed encoder allocates its tail and the engine canonical key.
        budget.retain_bytes(8 * std::mem::size_of::<u64>())
    }

    pub(in crate::compiler) fn recognizes(key: &Key) -> bool {
        key.kind == Self::KIND && key.params.first() == Some(&TAG)
    }

    pub(in crate::compiler) fn decode(key: &Key) -> Result<Self, String> {
        if !Self::recognizes(key) || key.params.len() != 4 {
            return Err("opaque declaration key requires tag, snapshot, flavor and owner".into());
        }
        let owner = ir::DeclId {
            snapshot: ir::SnapshotId::from_graph_word(key.params[1]),
            index: key.params[3],
        };
        match key.params[2] {
            0 => Ok(Self::Header(owner)),
            1 => Ok(Self::Validate(owner)),
            _ => Err("opaque declaration key has unknown flavor".into()),
        }
    }
}

impl ItemKey for OpaqueDeclaration {
    const KIND: u32 = crate::compiler::SOURCE_ADMISSION;

    fn params(&self) -> (u64, Vec<u64>) {
        let (flavor, owner) = match self {
            Self::Header(owner) => (0, owner),
            Self::Validate(owner) => (1, owner),
        };
        (TAG, vec![owner.snapshot.graph_word(), flavor, owner.index])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn opaque_admission_keys_preserve_owner_and_reject_malformed_flavors() {
        let owner = ir::DeclId {
            snapshot: ir::SnapshotId::standalone().unwrap(),
            index: 7,
        };
        let header = OpaqueDeclaration::Header(owner).key();
        let validate = OpaqueDeclaration::Validate(owner).key();
        assert_ne!(header, validate);
        assert!(
            matches!(OpaqueDeclaration::decode(&header).unwrap(), OpaqueDeclaration::Header(value) if value == owner)
        );
        assert!(
            matches!(OpaqueDeclaration::decode(&validate).unwrap(), OpaqueDeclaration::Validate(value) if value == owner)
        );
        let mut changed = header.clone();
        changed.params[2] = 2;
        assert!(OpaqueDeclaration::decode(&changed).is_err());
        changed = header.clone();
        changed.params.pop();
        assert!(OpaqueDeclaration::decode(&changed).is_err());
        changed = header.clone();
        changed.params[0] = u64::MAX - 2;
        assert!(!OpaqueDeclaration::recognizes(&changed));
        let denied = ir::PlanBudget::new(ir::PlanLimits {
            bytes: 0,
            ..Default::default()
        });
        assert!(
            OpaqueDeclaration::Header(owner)
                .charge_key(&denied)
                .is_err()
        );
    }
}
