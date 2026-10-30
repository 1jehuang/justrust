# Builtin test diagnostic skeleton: frozen v27/v28 evidence

Accepted source commit: `fe8df07f3`. Previous collector source: `3b13e250b`.
This bundle retains frozen root-owned bytes only. Packaging and offline audits do
not run native builds, engines, cargo, project tests or copied root commands.

## Scope and actual outcomes

The implementation introduces diagnostic-only builtin-test body eligibility.
Global outline, global INERT and codepatch behavior remain unchanged. Test
attribute spelling alone is insufficient: eligibility requires the resolved
`BuiltinAttrExpander::Test` in every selected active crate context. Unknown,
shadowed, unresolvable or ambiguous module contexts fail conservatively. The
source design and boundary details are in `docs/resident-test-diagnostic-skeleton.md`
at the repository root and in the frozen source-ready documentation copies.

V27 check/build passed but new native fixtures produced **3 pass / 5 fail**.
The complete failure journal is retained, including the missing synthetic proc
crate key and unresolved builtin-body positives. V28 repairs fixture setup only:
macro2 `pub macro test($item:item) {}` instead of macro_rules, an explicit child
module import, and removal of the premature explicit synthetic proc dependency.
The production prefix is byte-identical between those candidates. No positive
assertion, body-count requirement or macro provenance proof was weakened.

Root's v28 actual gates passed all **8** new fixtures, the existing focused gates
(**21 / 2 / 3 / 1 / 1 / 3**), and the full **359** native tests. Raw individual
native test journals are retained, not just filtered summaries. The existing
**700** benchmark regression tests passed in **9.624 s**. That regression run
checks harness correctness, not compiler workload performance.

The scoped resident one-line test-body edit observations are:

| Diagnostic | Old v26 median ms | New v28 median ms | Reference median ms |
| --- | ---: | ---: | ---: |
| First strict five-edit sessions | 107.675 | 86.353 | See each suite |
| Warm 42-edit counterbalanced | 99.509 | 83.863 | 111.151 |
| Warm 42-edit reverse order | 104.264 | 82.007 | 103.373 |
| Independent fresh candidate-first | 102.349298 | 82.189889 | Old 110.293067 / new 110.002826 |

The first strict comparison was -19.80% raw but only -5.84% reference-normalized,
below the requested 10% floor. The independent candidate-first comparison was
-19.6967% raw and -19.4848% normalized, above that floor **on this scoped fixture**.
Both results are retained. Machine contamination/noise remains unknown; no
statistical significance or general speedup verdict is established. These are
not the comprehensive session-weighted synthetic workload and do not justify
default promotion. Warm repetitions are not cold-support claims.

Actual enabled-instrumentation profiles show 373 selected bodies before and one
after on replicates 1 through 5. Replicate 0 separately contains a second
231-body collection on each engine. Preserve those separate groups; do not sum
them into a per-file result or invent a file/thread-ID association. All recorded
transport drops are zero. Collection phase spans are nested/inclusive, not an
additive wall-time budget. The skeleton proof itself costs roughly 1.4 ms versus
1.0 ms before. The profile is attribution, not disabled-runtime timing evidence.

## Portable contents and exact coverage

- `source-ready/v27/` and `source-ready/v28/`: full originals, frozen candidates,
  exact source patches, repair patch, SHA manifests and original readiness records.
  Pre-acceptance docs remain byte-exact, even where superseded by native results.
- `build/{v27,v28}/`: check/build logs, statuses, source identities, 17 frozen
  compiler input files and raw justrust build journals. Binary bytes and caches
  are omitted; their measured identities are retained.
- `unit/v27/`: failed eight-fixture run, all raw journals and root failure status.
- `unit/v28/`: eight actual native run journals, individual test outputs, focused
  summaries and full 359-test outcome, plus source-continuity checks.
- `suites/{rsb18,rsa24,rsa25,rsb19}/`: four independent strict sessions, **22 rows
  each / 88 total**. Each includes both engine and reference version, prepare,
  intentional type error/recovery, assertion failure/recovery and five body edits.
  Original source bytes, all stdout/stderr, suite hashes and identities remain.
- `warm/{forward,reverse}/`: **141 rows each / 282 total**. Each consists of
  126 edit rows (42 edits across old/new/Blaze), three prepares, six intentional
  failures and six recoveries. Every row remains, with no sample exclusion.
- `profile/`: **12 actual rows**, six old/new pairs, plus two startup prepares.
  Original fixture bytes, all outputs, actual traces, worker deltas and identity
  maps are retained. No inventory counter was converted into a synthetic event.
- `root/`: native admission, 700-regression log, exact accepted source commit patch,
  old engine identity, original orchestration scripts and both strict comparisons.
- `validate.py`: offline fail-closed coverage, raw outcome and identity checks.
- `test_validate.py`: 31 offline controls including isolated relocation and
  checksum corruption plus semantic mutations with freshly regenerated checksums.
- `SHA256SUMS`: exact file coverage except itself, generated `validation.json`
  and interpreter `__pycache__` files. Symlinks and escaping paths are rejected.

Historical absolute paths in raw payloads are provenance labels only. No original
scratch directory, repository checkout, installed engine or toolchain is required
by validation. The offline checker uses bundle-local relative paths. Copied root
scripts are **not** replay instructions and are never executed by validation.

## Replay without native workloads

```sh
python3 bench/results/resident-test-diagnostic-skeleton-20261030/validate.py
python3 -B bench/results/resident-test-diagnostic-skeleton-20261030/test_validate.py
```

A copied directory can run `python3 /copied/bundle/validate.py` from any working
directory. The relocation control does exactly that with Python isolated mode,
an empty working directory, a nonexistent PATH and a fresh empty HOME.

`collect.py` is a local reconstruction helper requiring the original scratch
payloads. It copies only frozen files, never resolves dependencies or runs tools,
and refuses to replace a previously copied payload with different bytes. After
intentional bundle maintenance, regenerate coverage using:

```sh
python3 bench/results/resident-test-diagnostic-skeleton-20261030/validate.py \
  bench/results/resident-test-diagnostic-skeleton-20261030 --seal
```

Sealing is separate from validation. Validation never silently accepts a changed
manifest or creates a missing raw output.

## What offline admission proves and does not prove

Checks include frozen compiled-source/handoff linkage, exact v27 failure retention,
individual native test counts and statuses, all runtime phases and edit marker
exact per-replicate values, exact four executed test names and the exact failing
test, type-error/assertion recovery oracles, all six warm stack permutations
repeated seven times in preserved order,
identical source edits across strict stacks/sessions, raw artifact hashes,
recomputed sample medians, source/toolchain/tool continuity, profile counter and
trace correspondence, exact positive 373-to-1 selected-body counts, separate
231-body groups, bounded inventory categories and zero drops.

This is a consistency/integrity audit of the trusted committed record. It is not
a cryptographic execution attestation: a party able to rewrite every raw payload,
identity, manifest and checker can fabricate a different self-consistent record.
It does not independently rerun or prove Rust language semantics, provide broad
workload coverage or establish statistical significance. Native correctness and
runtime observations belong to root's retained actual runs. No new native
measurement was substituted by a mock or offline fixture.
