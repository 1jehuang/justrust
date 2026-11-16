//! Scoped ownership for native live type state.
//!
//! This is the lifetime boundary needed by a future multi-revision compiler
//! executor, not a process-global cache and not an active driver mode. In
//! particular, creating this domain does not make an old `GlobalCtxt` reusable.
//! Sessions, source/hygiene identities and dependency validation must be brought
//! under the same executor before queries may reuse these values. The allocation
//! map is shared here so native AllocIds retain their meaning within the domain.
//!
//! The higher-ranked callback prevents a type or arena reference from escaping
//! its owner. No lifetime extension, permanent allocation, or serialized proxy
//! is involved. The caller must already be on a registered compiler worker.

use std::cell::Cell;
use std::sync::OnceLock;

use rustc_arena::TypedArena;
use rustc_data_structures::sync::WorkerLocal;

use super::{CommonTypes, CtxtInterners, GlobalCtxt, GlobalCtxtResources, OwnedOrBorrowed};
use crate::arena::Arena;
use crate::mir::interpret;
use crate::ty::{Ty, TypeckResults};

/// An arena and interner universe shared by the revisions in one scoped batch.
///
/// Its interners are deliberately private. Until a native executor establishes
/// the complete definition/source/allocation universe, arbitrary interned types
/// must not be transplanted into ordinary one-shot contexts.
pub struct ResidentTypeDomain<'domain> {
    arena: &'domain WorkerLocal<Arena<'domain>>,
    interners: &'domain CtxtInterners<'domain>,
    common_types: CommonTypes<'domain>,
    resources: &'domain GlobalCtxtResources<'domain>,
    context_cells: &'domain TypedArena<OnceLock<GlobalCtxt<'domain>>>,
    allocated_contexts: Cell<usize>,
}

// A revision-count bound is not a bound on arena bytes. The executor must also
// retire a batch under its measured memory budget before accepting another request.
const MAX_CONTEXTS_PER_DOMAIN: usize = 64;

/// Enter one live type ownership domain. The entire batch must finish before
/// this call returns. Resetting a domain means leaving this callback and dropping
/// all its arenas, not clearing a map while references remain live.
///
/// A returned `Ty` would borrow a local arena, so it cannot escape:
///
/// ```compile_fail,E0521
/// use rustc_middle::ty::with_resident_type_domain;
/// let mut escaped = None;
/// with_resident_type_domain(|domain| escaped = Some(domain.bool_type()));
/// ```
pub fn with_resident_type_domain<R>(
    f: impl for<'domain> FnOnce(ResidentTypeDomain<'domain>) -> R,
) -> R {
    with_resident_type_domain_and_owner((), |domain, _| f(domain))
}

/// Scope an external component's owned storage under the same lifetime as the
/// native type universe. The compiler interface can supply its own compiler
/// arena without introducing a dependency from `rustc_middle` back to it.
/// Context cells retire before this additional owner is destroyed.
///
/// ```compile_fail,E0521
/// use rustc_middle::ty::with_resident_type_domain_and_owner;
/// let mut escaped = None;
/// with_resident_type_domain_and_owner(String::new(), |_, owner| escaped = Some(owner));
/// ```
pub fn with_resident_type_domain_and_owner<Owner, R>(
    owner: Owner,
    f: impl for<'domain> FnOnce(ResidentTypeDomain<'domain>, &'domain Owner) -> R,
) -> R {
    let arena = WorkerLocal::new(|_| Arena::default());
    let hir_arena = WorkerLocal::new(|_| rustc_hir::Arena::default());
    let interners = CtxtInterners::new(&arena);
    let alloc_map = interpret::AllocMap::new();
    let common_types = CommonTypes::new(&interners);
    let resources = GlobalCtxtResources {
        arena: &arena,
        hir_arena: &hir_arena,
        interners: OwnedOrBorrowed::Borrowed(&interners),
        alloc_map: OwnedOrBorrowed::Borrowed(&alloc_map),
    };
    // Context cells drop before the resources and all native identity owners.
    let context_cells = TypedArena::default();
    f(
        ResidentTypeDomain {
            arena: &arena,
            interners: &interners,
            common_types,
            resources: &resources,
            context_cells: &context_cells,
            allocated_contexts: Cell::new(0),
        },
        &owner,
    )
}

impl<'domain> ResidentTypeDomain<'domain> {
    /// Borrow the batch's actual native arenas, interners and allocation map.
    /// Contexts may be dropped independently without discarding this universe.
    /// This alone does not validate source/DefId identity or query reuse.
    pub fn context_resources(&self) -> &'domain GlobalCtxtResources<'domain> {
        self.resources
    }

    /// Reserve one actual context cell under the factory's batch lifetime.
    /// Refuse before creating a session or acquiring incremental locks when the
    /// context-count bound is reached. The executor must retire this whole scope
    /// rather than clearing cells that can still be referenced by typed results.
    pub fn alloc_global_ctxt_cell(&self) -> Option<&'domain OnceLock<GlobalCtxt<'domain>>> {
        let count = self.allocated_contexts.get();
        if count >= MAX_CONTEXTS_PER_DOMAIN {
            return None;
        }
        let cell = self.context_cells.alloc(OnceLock::new());
        self.allocated_contexts.set(count + 1);
        Some(cell)
    }

    /// Number of context cells consumed, including failed context constructions.
    pub fn allocated_context_count(&self) -> usize {
        self.allocated_contexts.get()
    }

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
    pub fn alloc_mir(
        &self,
        value: crate::mir::Body<'domain>,
    ) -> &'domain crate::mir::Body<'domain> {
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

    use rustc_data_structures::intern::Interned;
    use rustc_data_structures::sync::{Registry, set_dyn_thread_safe_mode};
    use rustc_hir::CRATE_OWNER_ID;

    use super::*;

    fn on_worker(f: impl FnOnce() + Send) {
        // Native interners inspect synchronization mode. Initialize it once to
        // the thread-safe setting used by this parallel unit-test process.
        // This does not switch a previously initialized compiler mode.
        set_dyn_thread_safe_mode(true);
        // A fresh scoped worker avoids replacing another test's registry.
        std::thread::scope(|scope| {
            scope
                .spawn(|| {
                    Registry::new(NonZero::new(1).unwrap()).register();
                    rustc_span::create_default_session_globals_then(f);
                })
                .join()
                .unwrap();
        });
    }

    #[test]
    fn resident_domain_keeps_actual_types_and_typeck_values_between_revision_scopes() {
        on_worker(|| {
            with_resident_type_domain(|domain| {
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
            })
        });
    }

    #[test]
    fn resident_domain_unwind_drops_owner_and_allows_a_new_scoped_domain() {
        on_worker(|| {
            let failed = std::panic::catch_unwind(|| {
                with_resident_type_domain(|domain| {
                    assert!(domain.has_shared_primitive_identity());
                    let _ = domain.alloc_typeck(TypeckResults::new(CRATE_OWNER_ID));
                    panic!("discard this live batch");
                })
            });
            assert!(failed.is_err());
            with_resident_type_domain(|domain| {
                assert!(domain.has_shared_primitive_identity());
                assert_eq!(
                    domain.alloc_typeck(TypeckResults::new(CRATE_OWNER_ID)).hir_owner,
                    CRATE_OWNER_ID
                );
            });
        });
    }

    #[test]
    fn resident_resources_keep_native_owners_after_domain_view_is_dropped() {
        on_worker(|| {
            with_resident_type_domain(|domain| {
                let old = domain.context_resources();
                let new = domain.context_resources();
                assert!(matches!(&old.interners, OwnedOrBorrowed::Borrowed(_)));
                assert!(matches!(&old.alloc_map, OwnedOrBorrowed::Borrowed(_)));
                assert!(std::ptr::eq(old.arena, new.arena));
                assert!(std::ptr::eq(old.hir_arena, new.hir_arena));
                assert!(std::ptr::eq(&*old.interners, &*new.interners));
                assert!(std::ptr::eq(&*old.alloc_map, &*new.alloc_map));
                let old_ty = old.interners.intern_ty(crate::ty::TyKind::Bool);
                let old_typeck = old.arena.alloc(TypeckResults::new(CRATE_OWNER_ID));
                let old_hir_owner = old.hir_arena.alloc(CRATE_OWNER_ID);
                // The factory, not this movable domain view or a context's
                // resource adapter, owns the interner and allocation map.
                drop(domain);
                // These adapters are now borrowed directly from the factory.
                // Dropping the movable domain view cannot retire their owners.
                let next_ty = new.interners.intern_ty(crate::ty::TyKind::Bool);
                assert!(std::ptr::eq(old_ty.kind(), next_ty.kind()));
                assert_eq!(old_typeck.hir_owner, *old_hir_owner);
                let next_hir_owner = new.hir_arena.alloc(CRATE_OWNER_ID);
                assert!(!std::ptr::eq(old_hir_owner, next_hir_owner));
            });
        });
    }

    #[test]
    fn fresh_resources_own_independent_native_interners_and_maps() {
        on_worker(|| {
            let arena = WorkerLocal::new(|_| Arena::default());
            let hir_arena = WorkerLocal::new(|_| rustc_hir::Arena::default());
            let first = GlobalCtxtResources::fresh(&arena, &hir_arena);
            let second = GlobalCtxtResources::fresh(&arena, &hir_arena);
            assert!(matches!(&first.interners, OwnedOrBorrowed::Owned(_)));
            assert!(matches!(&first.alloc_map, OwnedOrBorrowed::Owned(_)));
            assert!(!std::ptr::eq(&*first.interners, &*second.interners));
            assert!(!std::ptr::eq(&*first.alloc_map, &*second.alloc_map));
            let first_ty = first.interners.intern_ty(crate::ty::TyKind::Bool);
            let second_ty = second.interners.intern_ty(crate::ty::TyKind::Bool);
            assert!(!std::ptr::eq(first_ty.kind(), second_ty.kind()));
        });
    }

    #[test]
    fn resident_resources_preserve_native_memory_and_alloc_ids_between_views() {
        on_worker(|| {
            with_resident_type_domain(|domain| {
                let first = domain.context_resources();
                let allocation = interpret::Allocation::from_bytes_byte_aligned_immutable(
                    &b"resident allocation"[..],
                    (),
                );
                let allocation = first
                    .interners
                    .const_allocation
                    .intern(allocation, |allocation| {
                        super::super::InternedInSet(first.arena.alloc(allocation))
                    })
                    .0;
                let memory = interpret::ConstAllocation(Interned::new_unchecked(allocation));
                let id = first.alloc_map.insert_for_test(interpret::GlobalAlloc::Memory(memory));
                let next = domain.context_resources();
                drop(domain);
                let recovered = next.alloc_map.get_for_test(id).unwrap().unwrap_memory();
                assert!(std::ptr::eq(memory.inner(), recovered.inner()));
                let next_id =
                    next.alloc_map.insert_for_test(interpret::GlobalAlloc::Memory(memory));
                assert_ne!(id, next_id);
                assert_eq!(next.alloc_map.get_for_test(id), next.alloc_map.get_for_test(next_id));
            });
        });
    }

    #[test]
    fn fresh_resources_do_not_share_alloc_id_meanings() {
        on_worker(|| {
            let arena = WorkerLocal::new(|_| Arena::default());
            let hir_arena = WorkerLocal::new(|_| rustc_hir::Arena::default());
            let first = GlobalCtxtResources::fresh(&arena, &hir_arena);
            let second = GlobalCtxtResources::fresh(&arena, &hir_arena);
            let first_ty = first.interners.intern_ty(crate::ty::TyKind::Bool);
            let id =
                first.alloc_map.insert_for_test(interpret::GlobalAlloc::TypeId { ty: first_ty });
            assert!(second.alloc_map.get_for_test(id).is_none());
            let second_ty = second.interners.intern_ty(crate::ty::TyKind::Char);
            let second_id =
                second.alloc_map.insert_for_test(interpret::GlobalAlloc::TypeId { ty: second_ty });
            assert_eq!(id, second_id);
            assert_eq!(
                first.alloc_map.get_for_test(id),
                Some(interpret::GlobalAlloc::TypeId { ty: first_ty })
            );
            assert_eq!(
                second.alloc_map.get_for_test(id),
                Some(interpret::GlobalAlloc::TypeId { ty: second_ty })
            );
        });
    }

    #[test]
    fn resident_context_cells_are_bounded_and_live_until_factory_retirement() {
        on_worker(|| {
            with_resident_type_domain(|domain| {
                let cells: Vec<_> = (0..MAX_CONTEXTS_PER_DOMAIN)
                    .map(|_| domain.alloc_global_ctxt_cell().unwrap())
                    .collect();
                assert!(domain.alloc_global_ctxt_cell().is_none());
                for pair in cells.windows(2) {
                    assert!(!std::ptr::eq(pair[0], pair[1]));
                }
                let resources = domain.context_resources();
                assert!(std::ptr::eq(resources.arena(), domain.arena));
                drop(domain);
                assert!(cells.iter().all(|cell| cell.get().is_none()));
                assert!(resources.interners.intern_ty(crate::ty::TyKind::Bool).is_bool());
            });
            with_resident_type_domain(|domain| {
                assert!(domain.alloc_global_ctxt_cell().is_some());
            });
        });
    }

    #[test]
    fn resident_additional_owner_shares_context_and_type_lifetime() {
        on_worker(|| {
            with_resident_type_domain_and_owner(
                TypedArena::<String>::default(),
                |domain, owner| {
                    let session_data = owner.alloc(String::from("owned session data"));
                    let context = domain.alloc_global_ctxt_cell().unwrap();
                    let resources = domain.context_resources();
                    let value = domain.alloc_typeck(TypeckResults::new(CRATE_OWNER_ID));
                    drop(domain);
                    assert_eq!(session_data, "owned session data");
                    assert!(context.get().is_none());
                    assert_eq!(value.hir_owner, CRATE_OWNER_ID);
                    assert!(resources.interners.intern_ty(crate::ty::TyKind::Bool).is_bool());
                },
            );
        });
    }

    #[test]
    fn resource_wrapper_drops_only_its_owned_value() {
        use std::cell::Cell;

        struct DropProbe<'a>(&'a Cell<usize>);
        impl Drop for DropProbe<'_> {
            fn drop(&mut self) {
                self.0.set(self.0.get() + 1);
            }
        }

        let drops = Cell::new(0);
        let owner = DropProbe(&drops);
        drop(OwnedOrBorrowed::Borrowed(&owner));
        assert_eq!(drops.get(), 0);
        drop(OwnedOrBorrowed::Owned(DropProbe(&drops)));
        assert_eq!(drops.get(), 1);
        drop(owner);
        assert_eq!(drops.get(), 2);
    }
}
