//! Native arithmetic failures retain the sealed source location of the CFG
//! operation. Infrastructure traps remain separate and keep their old policy.
use super::*;
use crate::compiler::ssa::PanicKind;
use std::sync::Arc;

pub(super) struct RuntimeTrap {
    pub branch: usize,
    pub kind: Option<PanicKind>,
    pub location: Arc<ir::RuntimeLocation>,
}

impl Emitter<'_> {
    pub(super) fn binary_panic_kind(&mut self, op: BinaryOp) {
        self.runtime_panic_kind = match op {
            BinaryOp::Add => Some(PanicKind::AddOverflow),
            BinaryOp::Sub => Some(PanicKind::SubOverflow),
            BinaryOp::Mul => Some(PanicKind::MulOverflow),
            BinaryOp::Div => Some(PanicKind::DivOverflow),
            BinaryOp::Rem => Some(PanicKind::RemOverflow),
            BinaryOp::Shl => Some(PanicKind::ShlOverflow),
            BinaryOp::Shr => Some(PanicKind::ShrOverflow),
            _ => None,
        };
    }

    /// Entry snapshot contract: rax=index and rdx=len, before address scaling.
    /// This only authorizes the exact source operation, never descriptor faults.
    pub(super) fn bounds_trap(&mut self) {
        let branch = self.jump(&[0x0f, 0x83]);
        if let Some(location) = &self.runtime_location {
            self.runtime_traps.push(RuntimeTrap {
                branch,
                kind: None,
                location: location.clone(),
            });
        } else {
            self.traps.push(branch);
        }
    }

    pub(super) fn runtime_panic_blocks(&mut self) -> Result<(), String> {
        for trap in std::mem::take(&mut self.runtime_traps) {
            self.patch(trap.branch, self.position())?;
            if let Some(kind) = trap.kind {
                self.panic(&ir::PanicMessage {
                    message: kind.message().to_owned(),
                    location: trap.location.diagnostic_location(),
                })?;
            } else {
                self.panic_with_bounds(
                    &ir::PanicMessage {
                        message: String::new(),
                        location: String::new(),
                    },
                    Some(&trap.location),
                )?;
            }
        }
        Ok(())
    }
}
