//! Scoped ownership for native live type state.
//!
//! This is the lifetime boundary needed by a future multi-revision compiler
//! executor, not a process-global cache and not an active driver mode. In
//! particular, creating this domain does not make an old `GlobalCtxt` reusable.
//! Sessions, source/hygiene identities, allocation IDs and dependency validation
//! must be brought under the same executor before queries may use these values.
//!
//! The higher-ranked callback prevents a type or arena reference from escaping
//! its owner. No lifetime extension, permanent allocation, or serialized proxy
//! is involved. The caller must already be on a registered compiler worker.

use rustc_data_structures::sync::WorkerLocal;

use super::{CommonTypes, CtxtInterners};
use crate::arena::Arena;
use crate::ty::{Ty, TypeckResults};

/// An arena and interner universe shared by the revisions in one scoped batch.
///
/// Its interners are deliberately private. Until a native executor establishes
/// the complete definition/source/allocation universe, arbitrary interned types
/// must not be transplanted into ordinary one-shot contexts.
pub struct ResidentTypeDomain<'domain> {
    arena: &'domain WorkerLocal<Arena<'domain>>,
    interners: CtxtInterners<'domain>,
    common_types: CommonTypes<'domain>,
}

/// Enter one live type ownership domain. The entire batch must finish before
/// this call returns. Resetting a domain means leaving this callback and dropping
/// all its arenas, not clearing a map while references remain live.
///
/// A returned `Ty` would borrow a local arena, so it cannot escape:
///
/// ```compile_fail,E0521
/// use rustc_middle::ty::with_resident_type_domain;
/// let escaped = with_resident_type_domain(|domain| domain.bool_type());
/// ```
pub fn with_resident_type_domain<R>(
    f: impl for<'domain> FnOnce(ResidentTypeDomain<'domain>) -> R,
) -> R {
    let arena = WorkerLocal::new(|_| Arena::default());
    let interners = CtxtInterners::new(&arena);
    let common_types = CommonTypes::new(&interners);
    f(ResidentTypeDomain { arena: &arena, interners, common_types })
}

impl<'domain> ResidentTypeDomain<'domain> {
    /// One actual interned compiler type, not a type name or encoded type record.
    pub fn bool_type(&self) -> Ty<'domain> {
        self.common_types.bool
    }

    /// Allocate an actual type-checking result under the batch owner. The
    /// `TypeckResults` may contain references with the domain lifetime, but
    /// cannot contain a shorter revision's references.
    pub fn alloc_typeck(&self, value: TypeckResults<'domain>) -> &'domain TypeckResults<'domain> {
        self.arena.alloc(value)
    }

    /// Allocate an actual MIR body under the same batch lifetime.
    pub fn alloc_mir(&self, value: crate::mir::Body<'domain>) -> &'domain crate::mir::Body<'domain> {
        self.arena.alloc(value)
    }

    /// Check native interning identity without exposing the unguarded interner
    /// to unrelated compiler sessions.
    pub fn has_shared_primitive_identity(&self) -> bool {
        self.common_types.bool == self.interners.intern_ty(crate::ty::TyKind::Bool)
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZero;

    use rustc_data_structures::sync::Registry;
    use rustc_hir::CRATE_OWNER_ID;

    use super::*;

    fn on_worker(f: impl FnOnce() + Send) {
        // A fresh scoped worker avoids replacing another test's registry or
        // changing dynamic synchronization mode process-wide.
        std::thread::scope(|scope| {
            scope.spawn(|| {
                Registry::new(NonZero::new(1).unwrap()).register();
                rustc_span::create_default_session_globals_then(f);
            }).join().unwrap();
        });
    }

    #[test]
    fn resident_domain_keeps_actual_types_and_typeck_values_between_revision_scopes() {
        on_worker(|| with_resident_type_domain(|domain| {
            let old_ty = { domain.bool_type() };
            let old_typeck = { domain.alloc_typeck(TypeckResults::new(CRATE_OWNER_ID)) };
            // End the first logical revision's stack, but not its domain owner.
            // Both values remain typed, directly addressable arena objects.
            let next_ty = { domain.bool_type() };
            assert_eq!(old_ty, next_ty);
            assert!(std::ptr::eq(old_ty.kind(), next_ty.kind()));
            assert!(domain.has_shared_primitive_identity());
            assert_eq!(old_typeck.hir_owner, CRATE_OWNER_ID);
            let next_typeck = domain.alloc_typeck(TypeckResults::new(CRATE_OWNER_ID));
            assert!(!std::ptr::eq(old_typeck, next_typeck));
            assert_eq!(old_typeck.hir_owner, next_typeck.hir_owner);
        }));
    }

    #[test]
    fn resident_domain_unwind_drops_owner_and_allows_a_new_scoped_domain() {
        on_worker(|| {
            let failed = std::panic::catch_unwind(|| with_resident_type_domain(|domain| {
                assert!(domain.has_shared_primitive_identity());
                let _ = domain.alloc_typeck(TypeckResults::new(CRATE_OWNER_ID));
                panic!("discard this live batch");
            }));
            assert!(failed.is_err());
            with_resident_type_domain(|domain| {
                assert!(domain.has_shared_primitive_identity());
                assert_eq!(domain.alloc_typeck(TypeckResults::new(CRATE_OWNER_ID)).hir_owner, CRATE_OWNER_ID);
            });
        });
    }
}
