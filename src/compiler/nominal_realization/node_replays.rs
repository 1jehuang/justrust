//! Node-local memo of whole ReadNominal source replays.
//!
//! A replay is an assumption-free source proof of one immutable fact under one
//! world, planning authority, normalization phase and depth policy. Callers
//! create many independent `NominalReplays` arenas, so the same fact used to be
//! re-proved from scratch in each of them. This memo lives in the polled
//! node's computation-local slot (`engine::with_node_local`), is dropped when
//! the node completes or fails, and is never reachable from another node.
//!
//! Every entry pins the fact allocation and the planning authority it names,
//! so a pointer identity can never be reused by a different payload. Both
//! successes and failures are memoized. A success at depth D also answers any
//! shallower occurrence, exactly like the per-arena memo. Failures answer only
//! the exact same depth.
use super::{Error, ir, reads::ReadNominal, traits};

struct Entry {
    fact: ReadNominal,
    world: u64,
    authority: ir::PlanAuthority,
    finalized: bool,
    depth: usize,
    result: Result<(), Error>,
}

/// Capacity failures of a computation's own scratch arenas depend on what the
/// arena already held, not only on the memo key, so they are never reused.
fn resource_bound(error: &Error) -> bool {
    [
        "limit",
        "exceeded",
        "overflow",
        "allocation",
        "resource",
        "budget",
    ]
    .iter()
    .any(|marker| error.0.contains(marker))
}

#[derive(Default)]
pub(super) struct NodeReplayMemo {
    // Keyed by the pinned fact allocation address. Entries in one bucket
    // still compare full identity.
    entries: std::collections::HashMap<usize, Vec<Entry>>,
}

impl NodeReplayMemo {
    fn matches(
        entry: &Entry,
        fact: &ReadNominal,
        world: &traits::ValidatedTraitWorld,
        finalized: bool,
    ) -> bool {
        entry.world == world.nominal_realization_world()
            && entry.authority.same_authority(world.budget().authority())
            && entry.finalized == finalized
            && entry.fact.same_replay_fact(fact)
    }

    /// A memoized answer for this exact fact, world, authority, phase and
    /// depth, or `None` outside graph polling or on a miss.
    pub(super) fn lookup(
        fact: &ReadNominal,
        world: &traits::ValidatedTraitWorld,
        depth: usize,
    ) -> Option<Result<(), Error>> {
        let address = fact.replay_fact_address();
        let finalized = world.owner_normalization_is_finalized();
        crate::engine::with_node_local(|memo: &mut Self| {
            let bucket = memo.entries.get(&address)?;
            for entry in bucket {
                if !Self::matches(entry, fact, world, finalized) {
                    continue;
                }
                match &entry.result {
                    Ok(()) if entry.depth >= depth => return Some(Ok(())),
                    Err(error) if entry.depth == depth => return Some(Err(error.clone())),
                    _ => {}
                }
            }
            None
        })
        .flatten()
    }

    /// Record a completed replay answer. Outside graph polling this is a
    /// no-op. Storage is paid by the replaying world's budget. A failure
    /// observed after this ledger latched a resource failure is not recorded:
    /// byte reservations can be released later, so it may not be a stable
    /// answer. Every other failure is a deterministic function of the key.
    pub(super) fn record(
        fact: &ReadNominal,
        world: &traits::ValidatedTraitWorld,
        depth: usize,
        result: &Result<(), Error>,
    ) {
        if crate::engine::with_current_meter(|meter| meter.is_none())
            || result
                .as_ref()
                .is_err_and(|error| world.budget().has_resource_failure() || resource_bound(error))
        {
            return;
        }
        // Retained for the node's lifetime: the slot is dropped at node
        // completion, outside the polled scope a reservation would refund.
        // An unpaid entry is never recorded. Failing to memoize only costs
        // repeated work, never a different answer.
        if world
            .budget()
            .retain_bytes(std::mem::size_of::<Entry>() + std::mem::size_of::<usize>())
            .is_err()
        {
            return;
        }
        let address = fact.replay_fact_address();
        let finalized = world.owner_normalization_is_finalized();
        let entry = Entry {
            fact: fact.clone(),
            world: world.nominal_realization_world(),
            authority: world.budget().authority().clone(),
            finalized,
            depth,
            result: result.clone(),
        };
        // A redundant entry is dropped after the slot is released.
        let redundant = crate::engine::with_node_local(move |memo: &mut Self| {
            let bucket = memo.entries.entry(address).or_default();
            for existing in bucket.iter_mut() {
                if Self::matches(existing, fact, world, finalized) {
                    match (&existing.result, &entry.result) {
                        // A deeper success subsumes shallower ones.
                        (Ok(()), Ok(())) => {
                            existing.depth = existing.depth.max(depth);
                            return Some(entry);
                        }
                        (Err(_), Err(_)) if existing.depth == depth => return Some(entry),
                        _ => {}
                    }
                }
            }
            bucket.push(entry);
            None
        });
        drop(redundant);
    }
}
