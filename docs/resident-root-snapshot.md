# Experimental resident classified root snapshot

Status: **REJECTED after nonreplication**, October 30, 2026. This was a
licensed Blaze-derived native-resident experiment, not the clean-room compiler.
The first final v19 pair showed -18.51% raw, but the candidate-first repeat
showed only -1.16% raw and +1.67% reference-normalized (worse). The apparent
above-floor result did not replicate. No robust, shipping, general-winner or
default-promotion claim is supported. Root restored all three production
paths separately after preserving exact candidate bytes. Only evidence and
this document are retained by this worker, with no native workload executed.

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
v19 savings claim is made. Root's new-identity full351 native suite passed,
but the final matched runtime replication failed the 10% improvement floor.
The cumulative signature-plus-coarse-snapshot source promotion was rejected.

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

The source-ready v19 snapshot is historical, not current production source.
Root restored main/codegen/signatures by a checked scoped reversal, not a
reset. This evidence worker did not restore or commit those production paths.
The full candidate can be reconstructed from the archived original bytes and
scoped patches, without reading the now-restored live compiler source.

## Retained evidence and exact arithmetic

`bench/results/resident-root-snapshot-20261030/` contains all twelve top-level
raw runtime sessions (six independent pairs), v16/v17/v18/v19 full native
build/test journals including all 351 names, original ready receipts and
full-source/patch lineage, all 36 instrumented trace rows and phase summaries,
and the exact pre-v19 namespace cleanup script/receipt. `validate.py` replays
raw gates, source hashes/patches, identities, runtime markers and medians.
`test_validate.py` exercises semantic mutations and a relocated copy. No
compiler commands are executed by either validator. The JSON audit includes
Decimal arithmetic from original JSON numeric lexemes, not rounded logs.

| Pair | Engine median before -> after (seconds) | Raw delta | Reference-normalized |
| --- | --- | --- | --- |
| v15 -> v16, RSB05/RSA11 | 0.114881208006409 -> 0.1085446599900024 | -5.52% | -5.00% |
| v16 -> v17, RSB06/RSA12 | 0.11774463900655974 -> 0.1121764150011586 | -4.73% | -14.40% |
| v4 -> v17, RSB07/RSA13 | 0.10634210900752805 -> 0.09845956500794273 | -7.41% | -7.19% |
| v17 -> v18, RSB08/RSA14 | 0.10810380399925634 -> 0.11217672500060871 | +3.77% | -2.78% |
| v4 -> v19, RSB09/RSA15 | 0.1158255369955441 -> 0.09438135699019767 | -18.51% | -21.76% |
| v4 -> v19, candidate-first RSA16 then RSB10 | 0.11851948899857234 -> 0.11714330599352252 | -1.16% | +1.67% |

Every row is a separate five-edit-sample diagnostic, not a pooled estimate.
Noise is unknown. Before the final v19 pairs, root issued native stop commands
for 34 completed exact private namespaces without deleting stores. This changed
resident-process memory pressure relative to historical pairs. Both final
fresh runs received the new policy. The receipt is native-stop output, not an
independent process-exit attestation. Do not attribute historical-to-final
differences solely to compiler source or generalize from the first pair.

The measured v17 instrumented worker-roots median changed from 10657us to 5888us,
while the matched uninstrumented total remained below the absolute floor.
v16 instrumented roots changed from 6562us to 7107us, and rejected v18 from 7454us
to 5802us. These are nested elapsed intervals, not CPU work, saved critical-path
time or total speedups. All twelve actual runtime rows per trace collection
are retained. The workload is filtered libtest on one pinned local project,
not the session-weighted comprehensive synthetic workload or a broad gate.

## Separate extended warm-store diagnostic

After rejection and production restoration, root reused the completed RSB09
and RSA15 private stores for a separate 42-edit diagnostic. Old v4, new v19
and Blaze ran all six stack permutations exactly seven times, with no sample
exclusion. Fifteen preparation/type-error/assertion/recovery invocations plus
126 timed invocations all passed. Complete source63, toolchain and executable
identities remained unchanged, and workload source was restored.

Raw medians were old 0.10035844549565809s, new 0.10059280800487613s and Blaze
0.10320122750272276s. Exact decimal replay of the 42 samples gives new versus
old **+0.2335254477693% slower**, not an attributable gain. Both old and new
were close to Blaze here. Noise remains unknown. This warm-store observation
reinforces the rejection but is not cold support, a general winner claim,
a significance estimate, a replacement for the six original fixed-five pairs,
or the comprehensive/default-promotion gate.

The separate `observations/interleaved-v19/` directory retains
runner/state/original source and all 282 stdout/stderr streams. Portable replay
independently checks ordering, all edit bytes/markers, failure and recovery
gates, raw test names/counts, tool/private-namespace binding and exact medians.
