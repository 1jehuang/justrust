# Native associated-item value retention, 2026-10-30 UTC

## Implemented, not promoted

`-Zjr-cache-associated-items=yes` independently enables native disk persistence
of `associated_items`. It is a tracked, default-off session option. The existing
small scalar query experiment is not enabled by this gate. Neither the shipping
client nor the installed compiler default changes.

The motivating existing macro-cache profile in `docs/reuse-ceiling.md` recorded
2,409 proven-green `associated_items` provider invocations. This is a candidate
selection count, not an estimate of removable elapsed time. The provider builds
an associated-item lookup by fetching item IDs and each descriptor. Persisting
the lookup avoids executing that provider on eligible proven-green nodes.

`AssocItems` previously had no codec. The new codec writes the semantic item
sequence in definition order, using the existing `AssocItem` encoder. It decodes
owned descriptors through the normal compiler DefId/Symbol translation and
reconstructs `SortedIndexMultiMap` with `AssocItems::new`. It deliberately does
**not** persist the sorted index: symbol numeric order can change across compiler
sessions. Returned references are allocated in the current session's arena by
the existing `arena_cache` plumbing. No pointer or reference survives a freed
compiler context.

Normal graph invalidation, query fingerprint verification, side-effect replay,
error handling and diagnostics remain in the existing native query engine.
When disabled, the query is neither admitted for persistence nor assigned a
promotion callback. The tracked flag separates incremental option identities.
The private compiler and cache remain local experimental inputs, never shipped
under an existing public toolchain identity or handed to stock tools.

## Exact paired evidence

One frozen matched compiler/sysroot, with only the tracked flag changed, passed
`bench/reuse-associated-accept.py` for 34 ordered cases. Receipts are committed in
`bench/results/associated-item-retention-20261030/acceptance.json`. The gate reuses
the existing query-retention compiler runner rather than substituting a model
of compiler behavior. Both modes enable `-Zincremental-verify-ich`.

Coverage includes unchanged/body edits and revert, owner addition/removal/order,
macros, async/const changes, cross-crate signature changes, associated type and
constant edits, trait item addition/order, RPITIT changes, symbol churn, and
recovery from six negative cases. These negative cases include missing and
duplicate associated items, ambiguous method resolution, a lint failure, a type
failure and a dependency mismatch. Exit codes and full diagnostics match after
normalizing only the per-mode source directory. All 28 successful cases execute
and produce the expected identical runtime output. Binary hashes are retained,
but byte-identical binaries are **not** asserted across tracked-option modes.
This is not exhaustive Rust semantic coverage. The complete Python benchmark
harness regression command passed 458 tests in 30.804 seconds, including the
three new associated-retention harness tests.

The original five existing native compiler regression fixtures also pass in
both modes with incremental fingerprint verification, through the real
`compiletest` binary:

* `explicit_supertraits_containing_assoc_item.rs`
* `issue-100521-change-struct-name-assocty.rs`
* `hashes/inherent_impls.rs`
* `hashes/trait_impls.rs`
* `hashes/trait_defs.rs`

Full successful compiletest JSONL receipts are committed beside the matrix.
Fixtures are unchanged. Their internal revisions run normally. These are five
selected tests per mode, not the complete 180-test incremental suite.

The native profiler count gate also passes. Its exact report, raw-profile hashes
and compiler/driver hashes are retained in `count-evidence.json`:

| Ordered epoch | Flag off green recomputes | Flag on green recomputes |
| --- | ---: | ---: |
| Unchanged | 3 | 0 |
| One-line body edit | 7 | 0 |
| Total observed `associated_items` | 10 | 0 |

Both native marker families are present and their query names resolve. Missing
selected-kind records are counted as zero only under those existing coverage
checks. These are observed provider-scope entries, not time saved or distinct
semantic queries. Warm preparation is not counted as reuse evidence.

After the final profiled body edit, observed `query-cache.bin` totals are 43,441
bytes off and 44,963 bytes on, a 1,522-byte increase. Both modes have 170,610-byte
dependency graphs and 667-byte work-product manifests. These filesystem byte
counts are retained in `storage.json`. They do not measure resident memory,
serialize/deserialize CPU or broad-workload storage cost.

The compiler reports `rustc 1.101.0-nightly (2967f9451 2026-10-14)`. The version
is not sufficient provenance for modified native sources. The frozen driver
SHA-256 is `beadc87fdfb8921e1c24a5f86a21ea43b0f2b36f8e407847b3cd6bcab54ebe71`.

## Reproduction and build recovery

Compiler bootstrap is necessary because this candidate changes native compiler
internals. The compiler-only incremental build took 284.2 seconds. Building its
matching standard library took 155.7 seconds. These are setup wall times, **not**
before/after compilation speed evidence.

```sh
cd toolchain/rustc
./x build --stage 1 compiler/rustc
./x build --stage 1 library
cd ../..
# Freeze a complete matching sysroot before concurrent bootstrap or root commits.
cp -a toolchain/rustc/build/host/stage1 "$JCODE_SCRATCH_DIR/associated-retention-20261030/sysroot"
python3 bench/reuse-associated-accept.py --rustc "$JCODE_SCRATCH_DIR/associated-retention-20261030/sysroot/bin/rustc" --output "$JCODE_SCRATCH_DIR/associated-retention-20261030/acceptance-v2"
python3 bench/reuse-associated-accept.py --rustc "$JCODE_SCRATCH_DIR/associated-retention-20261030/sysroot/bin/rustc" --output "$JCODE_SCRATCH_DIR/associated-retention-20261030/counts-v2" --profile-counts
bash bench/reuse-associated-incremental.sh "$JCODE_SCRATCH_DIR/associated-retention-20261030/sysroot" "$JCODE_SCRATCH_DIR/associated-retention-20261030/existing-v3"
python3 -m unittest discover -s bench -p 'test_*.py'
```

The new-output paths above must not already exist. The parent scratch directory
must exist. The incremental script expects the already built native compiletest
binary and native test helpers in the current compiler build tree.

Initial compiler-only acceptance failed due to missing std. A first bootstrap
regression attempt reused old incompatible std. After matching std was built, a
second bootstrap test invocation rebuilt the compiler because `--keep-stage 1`
does not preserve the stage0-produced compiler artifacts for a stage1 test. Root
commits changed its reported compiler identity again and old std was rejected
with E0514. These were setup failures, not waived test failures. The successful
gates use the frozen, matching sysroot and invoke the existing compiletest tool
directly, avoiding any later bootstrap identity drift. Early direct-run setup
mistakes (output root containment and repeated flag syntax) were fixed in the
committed script before the passing paired run.

## Remaining boundary

This is additional retained native query state, not a resident compiler or full
frontend reuse. It still reconstructs the lookup index when decoding, validates
the graph, and performs all required changed work and diagnostics. No broad
session-weighted suite pair was run for this independent candidate, so there is
**no end-to-end speedup claim and no default promotion**. A broad before/after
seed-1 comparison, cold/error/macro/dependency coverage, and memory/decode cost
measurements remain required before promotion. The larger HIR and lint-owner
opportunities remain separate and are not claimed solved by retaining this map.
