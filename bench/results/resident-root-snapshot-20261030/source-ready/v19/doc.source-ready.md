# Experimental resident classified root snapshot

Status: SOURCE READY v19 candidate, October 30, 2026. This is the licensed
Blaze-derived resident, not the clean-room compiler. This worker performs
source-only edits and offline evidence checks, not native builds or workloads.
Root owns native correctness, mechanism, and matched runtime retention gates.
No source promotion or shipping/default performance claim is made here.

## Boundary and dependency contract

`root_declarations` is a coarse tracked projection of the existing native
`module_declarations` / `body_declarations` graph. It replays the original
ordered worklist on the tracked thread, following ordinary and provided bodies
until both queues are empty. It does not substitute an outline, timestamp,
content hash, or manually invalidated revision key for Salsa. Body-declaration
queries still inspect lowered block maps, including nested items and expanded
macro items. A body-only change can revalidate this graph and equality-backdate
the complete ordered declaration result.

`root_snapshot`, keyed by the native base-db crate input, consumes that result
and classifies all roots on its own tracked thread. It stores ordered
functions, kept/all statics, in-provided functions, exported flags, rooted and
provided generic flags, and main. Reads of attributes, compile-time-only flags,
function/impl generics, main classification, and allocator trait/module/crate
origin remain native dependencies. All export predicates are read on the
tracked provider's thread. No result from a cloned worker database substitutes
for a dependency edge in the parent query.

`root_no_main` is a separate coarse boolean equality boundary around the
original root-parse predicate. Without it, an executable-body edit in the root
file would invalidate full classification simply because its parse changed.
It preserves the original simple-root-attribute predicate and its cfg/no_main
semantics, rather than silently broadening or narrowing them.

The snapshot contains no mutex, checkout-local shared-path cache, or mutable
state. Every new `ListedCrate` creates a fresh `shared_paths` mutex. Existing
`Listed` cache invalidation, files and built sets are unchanged. There are no
per-function wrapper queries.

## Bounded candidate lineage and cold/parallel risks

- v16 retained the original parallel `declared_in` / `lower` warmup and the
  original parallel export warmup outside the tracked classification provider.
  The provider independently re-read every dependency on its own thread.
- v17 removed only the unconditional outer export warmup. Export predicates
  are serial on a classification miss. Warm green reuse skips the export pass.
- v18 removed only the remaining outer `declared_in(db, krate)` warmup in
  `ListedCrate::new`, with a local comment update. `root_snapshot` already
  dependency-traverses declarations. Every tracked provider, read, original
  full oracle, and all four fixtures remain byte-identical to v17.

- v18 was rejected after root reported full351 native acceptance and 12 actual
  runtime trace rows, but the matched engine median increased from about
  0.108104s to 0.112177s (+3.77% raw, -2.78% reference-normalized). No improvement
  was established. Its exact bytes and patches remain archived, not erased.
- v19 restores main.rs to the exact preserved v17 bytes, then adds only
  `#[cfg(test)]` on the original parallel `exported` helper. The v17 parallel
  declaration/lowering warmup remains. All providers, reads, oracle bodies and
  fixtures are unchanged. This is experimental quality cleanup, not promotion.

Removing parallel warmup can regress cold, macro-heavy or parallel builds, as
the rejected v18 observation illustrates without establishing causation. v19
still classifies exports serially on misses. Its immutable snapshot costs
memory/vector cloning, and Salsa validation/synchronization is not free. No
v19 savings claim is made. Root must gate native semantics and matched total
wall time under its new build identity before deciding source retention.

Complete ra-jit caller audit before the v18 edit found additional production
`declared_in` callers at main.rs:1115, main.rs:3623, and mirdump.rs:69 (import at
line 26). `lower` also has the qualified `crate::lower` caller at outline.rs:356.
These helpers and every call site outside the constructor remain unchanged.
A complete export-symbol/caller search confirms that the original parallel
`exported` helper is used only by the cfg(test) full oracle. Calls named
`exported` inside the helper resolve to its local closure. v19 gates only this
helper with `#[cfg(test)]`, avoiding the non-test unused-function warning. Its
body and visibility, and the independent original oracle, are unchanged.

## Test-only mechanism and original oracle

Precise native filter: `resident_root_snapshot` (four unchanged tests).

- `resident_root_snapshot_reuses_and_invalidates_against_original`: provider
  counts require unchanged/body-only reuse and actual invalidation, comparing
  ordered snapshots to the independent complete pre-candidate classifier.
  Revisions cover nested items, macros, cfg/cfg_attr, compile-time-only attrs,
  statics, generics, main renaming and no_main.
- `resident_root_snapshot_tracks_allocator_dependency_crate`: dependency-file
  and origin-only native input changes must match the full allocator oracle.
- `resident_root_snapshot_tracks_parallel_export_warmup_reads`: 600 functions
  compare tracked serial classification with the original parallel oracle,
  changing an export attribute outside the first chunk on multicore hosts.
- `resident_root_snapshot_tracks_cfg_input_without_source_edit`: native cfg
  transitions update classification without source edits.

Candidates are requested before their oracles. `ListedCrate::original` does not
call the snapshot/declaration providers. The signature fixture and every
tracked provider/read remain unchanged through v17, v18 and v19. The thread-local
counter measures provider execution, not total lowering/validation work.

## Source freeze and reversal

Private directory:
`/home/jeremy/.jcode/scratch/blaze-native-engine-20261030/root-snapshot-v19/`.
It preserves exact pre-edit v18 main/doc/working-tree patch, the exact v17
main base, full v19 main/document/codegen/signature snapshots, export-caller
audit, exact v17-to-v19 and v18-to-v19 patches and SHA-256 hashes. All earlier
v16/v17/v18 freezes remain intact. The sibling
`root-snapshot-source-ready-v19.txt` is the explicit source handoff receipt.
Only the authorized v17 main base was restored, then the cfg(test) attribute
was added. No other source file was restored or changed.

Only main.rs and this document are source-worker owned. Check intervening
edits before any scoped patch reversal. Never reset unrelated working-tree
changes or erase the signature-IR candidate. Source retention is root's
separate decision. Native acceptance cannot be inferred from formatting or
from test-counter source alone.
