# Experimental resident classified root snapshot

Status: SOURCE READY v18 candidate, October 30, 2026. This is the licensed
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
- v18 removes only the remaining outer `declared_in(db, krate)` warmup in
  `ListedCrate::new`, with a local comment update. `root_snapshot` already
  dependency-traverses declarations. Every tracked provider, read, original
  full oracle, and all four fixtures remain byte-identical to v17.

v18 no longer schedules constructor declaration lowering through the old
parallel `lower` warmup. Cold graph traversal and export classification run on
the query's thread. This can regress cold, macro-heavy, or parallel builds.
The immutable snapshot still costs memory and vector cloning, and Salsa
validation/synchronization is not free. Avoiding a warmup is not evidence of
savings. No v18 speed claim is made. Root must gate cold/parallel behavior,
semantics, and matched total wall time before retaining the source.

Complete ra-jit caller audit before the v18 edit found additional production
`declared_in` callers at main.rs:1115, main.rs:3623, and mirdump.rs:69 (import at
line 26). `lower` also has the qualified `crate::lower` caller at outline.rs:356.
These helpers and every call site outside the constructor remain unchanged.
The original parallel `exported` helper is now used only by the cfg(test) full
oracle. It remains unchanged, including visibility. Its possible non-test
unused-function warning is pending root's quality gate, not suppressed here.

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
tracked provider/read remain unchanged through v17 and v18. The thread-local
counter measures provider execution, not total lowering/validation work.

## Source freeze and reversal

Private directory:
`/home/jeremy/.jcode/scratch/blaze-native-engine-20261030/root-snapshot-v18/`.
It preserves exact pre-edit v17 main/doc and working-tree patch, full v18
main/document/codegen/signature snapshots, caller audit, v17-to-v18 scoped
patches and SHA-256 hashes. Earlier `root-snapshot/` and `root-snapshot-v17/`
freezes remain intact. The sibling `root-snapshot-source-ready-v18.txt` is the
handoff receipt. No earlier base has been restored.

Only main.rs and this document are source-worker owned. Check intervening
edits before any scoped patch reversal. Never reset unrelated working-tree
changes or erase the signature-IR candidate. Source retention is root's
separate decision. Native acceptance cannot be inferred from formatting or
from test-counter source alone.
