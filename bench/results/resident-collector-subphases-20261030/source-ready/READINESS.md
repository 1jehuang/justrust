# READY: collector subphases v26 source-only handoff

Ownership of all six assigned production/documentation paths is RELEASED to skunk/root.
No further edits are planned from this lane.

Base: accepted `90164925ce021dbb56a735a7d99efacab261d732`.
Frozen on 2026-10-30, after direct rustfmt of only the five owned Rust files.
`git diff --check` passed for owned sources. All five full originals were compared
byte-for-byte with the accepted base via `git show | cmp` and matched.
The sixth path, `docs/resident-collector-subphases.md`, did not exist at the base.

## Artifacts

- `originals/`: complete original contents of the five existing owned files.
- `frozen/`: complete final contents of all six assigned files, original path layout.
- `source.patch`: five-file diff against accepted base plus the new documentation diff.
- `SHA256SUMS`: SHA-256 inventory for originals, frozen files and source.patch.
- Patch SHA-256: `fab82069606ddfcda7f900b0845337b0ebc68bf138f92fc84a08d23a486230ac`.

## Delivered design

A new optional HIR collector API produces a caller-owned inventory of at most 16
stable category rows, each with actual RAII elapsed nanoseconds and operation calls.
The fixed local counter array uses Cell references, no locks, no per-body allocation
or transport events. Existing rows merge by label. None initializes no inventory
and calls neither entry nor exit clock. Old HIR and IDE APIs explicitly pass None.

The existing timed IDE API has an extra optional inventory argument. Its existing
five exact actual `(static label, Instant, Instant)` phase intervals remain intact.
Main prints the inventory once under the existing startup telemetry flag, while
transporting the five actual outer intervals through the existing bounded collector
on the same thread. No synthetic HIR timeline events are emitted.

Categories distinguish naming, signature setup/generic/field work, wellformed,
impl checks, selected-body expression/inference/unsafe/validation/borrowck work,
and signature anonymous-const inference/borrowck. Inclusive nesting and limits of
attribution are documented in the new owned documentation file. No caching or
semantic optimization is included.

## Gates intentionally not executed

No build, test, benchmark, dependency resolution, lock update, install, commit or
push was performed. Native verification and actual measurement belong to root.
Only source inspection, direct rustfmt, diff whitespace inspection, original/base
byte comparison and frozen artifact hashing were performed.

The new distinct `resident_collector_subphases` filter contains six IDE tests and
two HIR helper tests. Actual APIs compare complete Debug diagnostics and exact
body-callback visitation under all/none/selective selection, enabled and disabled
inventory, and disabled diagnostic filtering. Rich raw HIR fixtures exercise
signature anonymous consts, naming, impl checks and nested modules. Counter labels,
positive operations and selected-body counts are asserted. The clock helper uses
a panicking clock closure on the disabled path. The repeated-scope helper tests
bounded merging separately from actual API differential coverage.

All five existing `resident_semantic_subphases` tests remain present and their
five-phase interval expectations are unchanged. Their calls were updated for the
extra optional inventory parameter.

No new speedup or category-dominance claim is made. Root should run native gates
and representative telemetry before drawing a caching or critical-attribution
conclusion. Unrelated FINDINGS, data/runs, .wrangler/cache and bench/results work
was not modified by this lane.
