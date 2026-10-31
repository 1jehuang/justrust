# Resident declared graph query (v69)

## Status and ownership

Source-only candidate against accepted `afa7d609519aa1ba05a6fa71770f12aa8ae76011`.
Only `toolchain/resident/crates/ra-jit/src/main.rs` and this new document are owned.
No native build, test, probe, build lock, installation, or commit was performed
for this candidate. The coordinator owns correctness and latency gates after
release. There is no measured speedup and no default promotion.

Motivation supplied by the coordinator: accepted v68 OFF recovery worker
`home-empty-fast-v68-off-repeat-seven/04-type-recovery-worker-0.log` reports
roots 4.0s, eager scope/body listing, and core's 6490-body listing twice.
This is motivation, not evidence that this candidate is faster.

## Selection and output

`RA_JIT_DECLARED_QUERY=1` opts in. Every other value, including unset, empty,
`0`, `true`, `01`, and `1 `, is OFF. A process `OnceLock` freezes selection
on the first `declared_in` call, even if OFF. Restart the daemon and workers
when changing it. This is not a per-request switch.

OFF calls `declared_in_original`. Its algorithm body is byte-for-byte the
accepted `declared_in` body, including the parallel `lower` prewarming and
listing telemetry. Only its function name changed.

ON calls the native Salsa `crate_declarations` tracked query using
`ide_db::base_db::Crate`, the actual Salsa input ID. The provider returns a
reference to the complete `Declared`; the existing value-returning caller
clones it. Nothing is sorted, deduplicated, reconstructed from a decoder,
or dropped. `functions`, `statics`, `in_provided`, the exhausted worklists,
and `under_provided` all participate in existing `PartialEq`/`Eq`.
Expansion visits ordinary then provided worklists in the same order as OFF,
including work appended to the next pass. The final traversal-state flag
remains false exactly as in OFF's `extend`-based algorithm.

No persistent graph store, v39 decoder cache, v47 scope-only cache, revision
map, mutable global graph cache, or correctness bypass is introduced.
The immutable selection and one-time marker control do not cache query values.

## Dependency and cycle audit

The provider explicitly reads `crate_def_map`, clones `module_declarations`,
and recursively expands the returned body IDs through `body_declarations`.
Those tracked calls, and their results, remain visible to Salsa.

* `module_declarations` walks module children, definition maps and scopes.
* `Declared::in_scope` reads declaration order, unnamed constants, impls,
  trait/impl associated items, function containers, and function signatures
  through `is_bodiless_intrinsic`. This retains signature/attribute reads.
* `body_declarations` reads `Body::of`, then block definition maps, modules,
  and each block's scope. It includes constants, statics, closure blocks,
  macro expansions and provided-method nested declarations, not only functions.
* The owned `Body::of` is itself tracked and reads `Body::relative` or
  `Body::with_source_map`. No `untracked`, synthetic dependency, or
  `report_untracked_read` workaround is used.
* Existing lower-level library providers cannot call this new binary-local
  provider. No existing query was redirected to `declared_in`. The final
  provider calls fragments, never itself. Worklist recursion follows the
  original nested-item traversal, not recursive final-query calls. Existing
  name-resolution/body cycle handling is unchanged. Dynamic cycle behavior
  still needs the coordinator's real execution gates.

The provider receives `&dyn HirDatabase`, not a cloneable RootDatabase
snapshot. It therefore lowers sequentially through `body_declarations`,
without off-stack or cross-snapshot prewarming. No sound native parallel
provider is established by this patch. OFF retains its original parallel
lowering. ON cold-query cost might regress substantially and must be measured.
Warm validation can still lower/revalidate fragments before finding their
outputs equal. A final provider cache hit does not prove zero lower-level work.
Signature/body edits may leave declaration IDs equal and allow green reuse;
provider reruns are not forced to manufacture activity.

## Observability and source tests

Optional `RA_JIT_DECLARED_QUERY_MARKER=/path` appends
`declared-query-provider-entry pid=<pid>` once per process, inside the actual
provider. File-open/write errors are ignored. It proves provider entry only,
not a hit, successful completion, revision count, or a speedup. The first
provider entry consumes the one-time attempt even if no path is set or I/O
fails. It is diagnostic output only, never a database dependency or input to
selection/results.

`cfg(test)` TLS `DECLARED_QUERY_PROVIDERS` increments exclusively inside the
real tracked provider. Tests call it on real RootDatabase fixtures without
mutating process environment and without dummy event names. They compare
full `Declared` equality, including vector order and all fields, against both:

1. The preserved OFF algorithm, retaining parallel `lower`.
2. A direct independent module/scope/body walker using neither tracked
   declaration fragment query nor the final graph query.

Fixtures require cold entry 1 and unchanged repeat entry 0. They include
nonempty named nested items, provided methods and associated constants,
ordinary constants/statics, closures, cfg attributes, macro-generated items,
signature-only edits, removals/recovery through successive revisions, and
cross-crate dependency macro/signature changes. Graph-changing revisions
require actual provider execution. Body-only and signature-only revisions
compare full oracle equality without forcing provider counts when Salsa can
backdate unchanged outputs. An unrelated crate's body edit must retain the
app's green graph. A dependent macro must actually emit its added named item,
so that dependency invalidation cannot pass vacuously.

These tests are written, not executed. Pure flag tests cover exact selection;
process-first-use behavior also needs separate fresh-process ON/OFF gates,
not an unsafe environment mutation in concurrent unit tests.

## Coordinator gates after source release

Use identical accepted source inputs and fresh daemon/worker process trees
for OFF and ON. Run the `declared_query_` real-database fixtures and existing
`resident_graph_reuse_tracks_nested_items_and_macro_edits` fixture, then full
native correctness/root gates. Confirm actual provider-entry markers only
on ON processes, including worker inheritance. Keep all diagnostics visible.
Measure cold traversal separately from repeat and edit/recovery traversal,
including the supplied recovery workload and core duplicate-listing case.
Compare cold parallel OFF against cold sequential ON explicitly. Run broader
representative workloads before any promotion, and do not infer latency from
provider counts or missing listing lines. Preserve binaries, logs, source
hashes and gate commands under the coordinator's root.

Release artifacts are under
`<PRIVATE_EXPERIMENT_ROOT>/declared-query-v69`:
`main.original.rs`, `main.candidate.rs`, `resident-declared-query.candidate.md`,
`candidate.patch`, `SHA256SUMS`, `source-checks.txt`, and `RELEASE`.
