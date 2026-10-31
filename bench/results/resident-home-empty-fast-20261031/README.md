# Rejected empty-home fast path v68 evidence

**Decision: rejected for performance, source restored, no promotion.** Native
correctness and all 28 strict runtime rows passed. The actual enabled branch
was entered, but two paired OFF/ON loops did not establish a repeatable useful
scoped latency improvement. Noise is unknown. This bundle claims neither a
speedup nor a causal regression. See `docs/resident-home-empty-fast.md` in the
repository for the phase table, hypothesis, actual gates, and restoration.

## Contents and provenance

- `observations.json`: fixed-field derivatives of four seven-phase actual
  runtime cases, original allowed stdout, exact named test contracts, statuses,
  fresh nonce/type/assertion witnesses, safe controls, source identities,
  cumulative worker marker snapshots, and complete SHA/byte provenance for
  selected private originals. It does not publish raw private state/logs.
- `inventories/source63.json`, `toolchain4020.json`, and
  `native-source3757.json`: full relative inventories. All four runtime cases
  share unchanged before/after workload and pinned-toolchain identities. The
  phase-specific source edits are retained in each row's input inventory.
- `native/codegen-results.json`: all 56 named passing results, the actual
  ignored NO_HOME fixture line, and original full summary. `resident-results.json`
  contains all 364 actual named passing results and original summary.
- `native/fixtures-{unset,off,on,no-home}.json`: four actual separate-process
  control journals. Unset, OFF, and ON each passed the three ordinary real
  RootDatabase fixtures, with one ignored guard fixture. NO_HOME ran that
  ignored-by-default fixture alone and passed. Only ON emitted a branch marker.
  `native/fixture-launch-witness.json` is root's explicitly post-hoc tool-call
  witness for exact environment/arguments/exits/run IDs, not a contemporaneous
  worker-generated launch receipt. Independent original journals prove the
  actual named test execution and marker output. No rerun created this witness.
- `frozen/baseline/`, `candidate/`, and `live-before-restoration/`: byte-exact
  full Rust and manifest snapshots. Candidate/live proposal documents, the
  original full patch, and path-bearing release/source receipts are sanitized
  text derivatives. Original SHA/size provenance is preserved separately.
  `change-home.rs.patch` and `change-Cargo.toml.patch` replay with exact context.
- `frozen/lock.patch.txt`: root's two existing dependency edges only, ide-db and
  test-fixture, with no new packages. Root separately restored this lock.
- `analysis/`: root's actual successful rejection output and a sanitized
  derivative of its exact historical pair-analysis source.
- `oracles/`: fixed historical original oracle/parser sources, actual runtime
  driver, and native gate script as text. Replay never imports the execution
  drivers or invokes their process/build functionality.
- `checks/restoration.json`: the actual worker exact backup comparisons and
  scoped HEAD-diff restoration witness. This is a point-in-time receipt.

The marker `ra-jit: home-empty-fast-v68 branch` is cumulative once per process.
Repeated ON snapshots repeat that earlier line. They do not establish
per-phase branch work, avoided allocation counts, or latency attribution.
The candidate flag was exact `1`, process-latched, and default OFF. Its original
NO_HOME and caller-bounds guards were retained ahead of the specialization.
The bytewise source reconstruction checks that the disabled/unhandled original
path was unchanged. The candidate is archived here, not shipped.

## Portable offline replay

From any working directory, including after relocation:

```sh
python3 -B /path/to/bundle/validate.py --manifest
python3 -B /path/to/bundle/test_offline.py
```

Replay uses only Python's standard library and packaged files. No Rust, Cargo,
justrust, engine, daemon, toolchain, Git, or network job is launched. The original
hash-bound pure parser, contract, success, workload-validity, and matrix-oracle
functions are extracted by AST without changing their bodies or importing
module-level execution code. Raw selected text is freshly reparsed into the
original tuple-valued results. The original equality oracle is not weakened to
accept JSON lists. An independent exact named/count/status oracle also checks
all 28 outcomes, followed by full native result sets, four controls, source
identities, exact patch replay, pair arithmetic, rejection, and restoration.

Observed controls: eight Python tests passed, including 42 semantic negative
mutations, coherent raw/parsed wrong names, exact context rejection, a rehashed
failed-native-result mutation, and a coherent elapsed-time plus matching root
analysis/arithmetic rewrite. The latter passes arithmetic semantics but fails
the fixed historical observation commitment even after all mutable manifest
hashes are recomputed. An unrelated-directory relocation passed with empty HOME,
empty PATH, and Python `-I -B`, leaving the empty HOME untouched. These controls
are evidence verification, not additional native acceptance or performance work.

`SHA256.json` inventories the complete bundle. Independent fixed digests in
`validate.py` bind the selected historical observations, source snapshots, and
pure oracle sources. These are historical integrity commitments, not an
external cryptographic authenticity/signature guarantee.

## Privacy selection

`export_private.py` is a fixed allowlist recipe. Regeneration requires existing
private artifacts and is deliberately not needed for portable replay. It selects
eight explicit non-path environment values and five explicit controls, never the
ambient environment. Twelve path-bearing environment entries per runtime row
are omitted, not republished. Runtime stdout must match full-line test/witness
and summary grammars. Only the exact observed E0308 header, typed nonce line,
assertion witness line, and fixed branch marker are selected from stderr/worker
logs. Unexpected stdout aborts export. Private source/script constants are
replaced by labeled placeholders only for explicitly selected text derivatives.

Excluded: ambient environment, raw private state, raw worker/daemon logs,
private home/cache roots, credentials, telemetry, executables, targets, and
unrelated files. Relative toolchain inventory names do not expose installation
roots. The evidence-only assembly performed no native jobs or installs.
