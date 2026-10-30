# Experimental resident classified root snapshot

Status: SOURCE READY v17 candidate, October 30, 2026. This is the licensed
Blaze-derived resident, not the clean-room compiler. No native build, test,
workload, commit, or speed measurement was performed by this worker. Root owns
native correctness, mechanism, and matched runtime gates before retention.

## Boundary and dependency contract

`ListedCrate::new` still executes the original `declared_in` traversal and its
parallel `lower` warmup, outside the tracked query. v17 removes only the
unconditional outer parallel `exported` warmup. Export classification now runs
serially on snapshot misses, on the tracked provider's thread. A green snapshot
avoids that export pass entirely. The original parallel export helper remains
unchanged for the independent test-only full classifier. Its cloned-database
reads are never used as parent-query dependencies.

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
origin remain native dependencies. All export predicates are read here on the
tracked provider's thread. No result from a worker thread is trusted as a
substitute dependency edge.

`root_no_main` is a separate coarse boolean equality boundary around the
original root-parse predicate. Without it, an executable-body edit in the root
file would invalidate full classification simply because its parse changed.
This preserves the original predicate, including its treatment of simple root
attributes, rather than silently changing cfg/no_main semantics.

The snapshot contains no mutex, checkout-local shared-path cache, or other
mutable state. Every new `ListedCrate` creates a fresh `shared_paths` mutex.
Existing `Listed` cache invalidation, files, built sets, diagnostics, statistics,
and original parallel warmup implementations are unchanged.

## Deliberately bounded scope and performance risks

This is a safe narrower optimization than caching the entire existing
`ListedCrate::new` operation as-is. Its parallel export/body reads cannot safely
be hidden in a single tracked provider. We cache immutable classification,
while leaving the existing parallel graph/lowering warmup outside it.
There are no per-function wrapper queries.

Cold classification repeats warm graph traversal on the tracked thread and
now classifies exports serially without the previous parallel export warmup.
This can regress cold or export-heavy parallel builds even if warm green reuse
improves. It also allocates an additional immutable snapshot and clones vectors
into `ListedCrate`. Parallel crate listing can retain duplicate graph warmup
work and add Salsa synchronization. Body-only edits still pay original
declaration/lowering warmup, native dependency validation, and vector cloning.
Provider reuse is not itself a timing win.
There is no v17 speed claim and no default promotion. Root must inspect both raw
and normalized matched timings, cold behavior, and parallel behavior.

## Test-only mechanism and original oracle

Precise native test filter: `resident_root_snapshot` (four tests).

- `resident_root_snapshot_reuses_and_invalidates_against_original`: a
  thread-local provider counter demands unchanged and body-only reuse, and
  provider execution on classification edits. Ordered snapshots are compared
  after every revision with `ListedCrate::original`, a separate test-only copy
  of the complete pre-candidate classifier. Revisions cover nested items,
  macro-produced functions/statics, cfg/cfg_attr, compile-time-only flags,
  kept/exported statics, impl/nested generics, main renaming, and no_main.
- `resident_root_snapshot_tracks_allocator_dependency_crate`: dependency-file
  changes and origin-only native crate input changes must change allocator
  rooting and match the full oracle without an app source edit.
- `resident_root_snapshot_tracks_parallel_export_warmup_reads`: 600 functions
  exercise multiple export chunks in the unchanged original oracle on multicore
  hosts, changing an attribute outside the first chunk and comparing the
  candidate's serial tracked classification with the full oracle. The fixture
  is unchanged in v17.
- `resident_root_snapshot_tracks_cfg_input_without_source_edit`: native cfg
  input transitions must update the full snapshot without a source edit.

Each candidate is requested before its original oracle. The original classifier
does not call `root_snapshot` or `root_declarations`. Existing signature-IR
candidate sources and the `resident_signature_flags` fixture are not modified.
The counter measures only provider execution on the requesting test thread,
not total lowering or dependency-validation work.

## Source freeze and reversal

Private scratch directory:
`/home/jeremy/.jcode/scratch/blaze-native-engine-20261030/root-snapshot-v17/`.
It contains exact pre-edit v16 `main.v16.rs` and `doc.v16.md`, the preserved
v16 working-tree patch, full v17 main/document/codegen/signature snapshots,
scoped v16-to-v17 patches, and SHA-256 hashes. The original earlier freeze
remains in the sibling `root-snapshot/` directory without being restored.
The sibling `root-snapshot-source-ready-v17.txt` is the v17 handoff receipt.

Only `toolchain/resident/crates/ra-jit/src/main.rs` and this new document belong
to this worker. For reversal, apply the private scoped patch in reverse after
checking for intervening edits. Do not reset unrelated working-tree changes or
remove the signature-IR candidate. Native acceptance and runtime results must
be attached by root, not inferred from formatting or provider-count source.
