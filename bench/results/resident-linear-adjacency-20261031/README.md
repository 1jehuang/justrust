# Rejected native linear-adjacency experiment, 2026-10-31

**REJECTED for shipping. Production restored. No elapsed speed claim, fresh-phase
attribution, or default promotion.** This is licensed Blaze-derived resident
work, not clean-room work. This directory is a portable historical evidence
bundle, not an installed implementation. The evidence lane ran no native build,
test, benchmark or stop command.

## Decision

v50 replaced repeated `named().nth(visit.seen)` prefix scans in native cache DFS
with a monotonic raw `funcs` cursor. Non-named targets were consumed once while
preserving exact named-edge order, duplicates, SCC settlement, links, all native
counters and fallback. v51 added a default-OFF feature that actually ran BOTH
original and candidate selectors on each identical reached visit. It counted
actual raw inspections before filtering and asserted exact Option equality
before DFS. It added no named census or per-edge atomics/events. The frozen
source-only proposals are fully retained, but neither is promoted.

Root's actual original-workspace check/build and full native tests passed:

| Identity | Codegen passed | Resident passed | Feature |
| --- | ---: | ---: | --- |
| accepted v43 baseline | 53 | 360 | diagnostic absent |
| v50 raw cursor | 54 | 360 | diagnostic absent |
| v51 exact shadow attribution | 55 | 360 | `hir-ty-codegen/native-adjacency-diagnostic` |

All suites had zero failed/ignored/filtered tests. v50's real native store-backed
differential fixture compared full original DFS results, all19 counters,
settlement and ordered native-symbol callbacks on mixed/high-fanout/SCC/failure
graphs. v51's native scan fixture asserted 49792/768 original/candidate inspections
for mixed high-fanout exhaustion and 12610/386 for early failure, with exact
visits/calls/yields/exhaustion/aborts. These passing gates are fixture evidence,
not an arbitrary-language correctness proof.

All28 actual uninstrumented strict runtime calls passed. Percent changes are
`100 * (candidate / baseline - 1)`, positive means slower:

| State | First pair | Candidate-first reverse pair |
| --- | ---: | ---: |
| cold-prepare | +22.1083% | -22.0209% |
| repeated-noop-1 | +3.2883% | +4.5698% |
| manifest-feature-add | +4.2819% | -3.2114% |
| type-error | +4.0689% | -3.3175% |
| type-recovery | -6.6256% | +3.8428% |
| assert-failure | +5.3937% | -3.7240% |
| assert-recovery | -15.4684% | -19.5029% |

Most major states varied. The lower assertion-recovery times alone did not
establish a repeatable useful overall latency win. Noise remains **unknown**:
daemon/background CPU and OS caches were not controlled by waited-child CPU.
These are scoped counterordered pairs, not the comprehensive session-weighted
synthetic gate. Root rejected v50/v51 for shipping.

## Actual scan attribution, not elapsed speed proof

All7 strict diagnostic calls passed. Its output always marks
`native-adjacency-dual-v51 timings_invalid=true`. Diagnostic timings are invalid
for speed claims and are never included in the28 uninstrumented comparison.

| Worker snapshot captured after | Candidate inspections | Original inspections | Original/candidate | Max raw fanout |
| --- | ---: | ---: | ---: | ---: |
| manifest-feature-add | 109830 | 416910 | 3.795957 | 79 |
| type-error | 109692 | 417574 | 3.806786 | 79 |
| type-recovery | 147623 | 671534 | 4.548979 | 151 |

These counts are actual same-visit selector work. **Snapshots overlap or reset.
Do not sum them or describe them as fresh per-phase work.** Assertion-failure
and assertion-recovery repeated the type-recovery visible report counts
unchanged. They do not attribute the lower assertion-recovery elapsed times to
fresh cache DFS work. Zero visible reports do not independently prove zero work.

The initial `validate-linear-adjacency-v51.py` failed on its monotonic cumulative
snapshot assumption. Its original script and zero-byte output remain under
`provenance/`. Its failure must not be overwritten by the corrected result.
The separate `validate-linear-adjacency-v51-snapshots.py` and its original
snapshot-only JSON passed. Offline replay executes both exact archived scripts
against the relocated raw snapshots: the first must reproduce
`worker report snapshots reset or lost, cannot attribute phase deltas`, and the
second must reproduce the complete corrected JSON. That offline failure replay
is not misrepresented as the original native task stderr.
`provenance/initial-analysis-failure.json` explicitly attributes the historical
failure to the coordinator report and states that the original task stderr
journal was not available in this bundle. The empty original output and failed
script are retained byte-for-byte.

## Retained evidence and identities

- `frozen/v50`: complete accepted original, full candidate, proposal doc, patch,
  SHA manifest. Original cached.rs SHA starts `11543e4a`, candidate `3cba6e0b`.
- `frozen/v51`: complete original/candidate Cargo.toml, cached.rs and doc,
  incremental patch, readiness, SHA manifest and all exact live-before-restoration
  bytes. Candidate cached.rs SHA starts `d5d4d88e`.
- `native/candidate-{build,unit}-{v43,v50,v51}`: original native logs, exit files,
  all-source3757 manifests/checks, production-lock checks, native meta/output/
  summary/units journals, engine version and whole-binary SHA receipts.
- `raw/`: every original state.json, stdout/stderr, worker-log snapshot and input
  backup for the four v50 cases and one v51 diagnostic case. `rows35.json` is
  only an exact index, not new measurements.
- `snapshots/source63.tar.gz`: exact pinned63 workload source bytes, checked
  against every original source manifest and reconstructed edit recipe.
- `provenance/`: original launchers, probe, failed and corrected analyses and
  results, production Cargo.lock, cleanup script. Original paths are retained
  verbatim as historical metadata, never dereferenced by portable replay.
- `cleanup/`: actual original post-measurement receipt and stdout/stderr for
  eight exact completed namespaces, including four block-scope cases and four
  adjacency cases. It records stop commands, not store deletion. This lane did
  not execute that script. The receipt does not claim a v51 stop.
- `restoration.json`: exact two-path original/HEAD cmp checks at
  2026-10-31T05:36:34Z, release at05:36:45Z, with original full-file hashes.

The whole3757 native source delta v43 to v50 is only cached.rs, and v50 to v51
only cached.rs plus the feature declaration in Cargo.toml. Production lock
remained unchanged. All raw runs join the exact native binary identity before
and after and the same complete4020-file pinned nightly toolchain identity,
including channel manifest SHA
`34f3d9bf04640a082e561310f3f258846397f4cf64a2bcfebcea96ebf18e2572`.
No binary, private cache or complete toolchain distribution is embedded.

All three engines report Blaze-derived origin
`5d4f78f7577cf7170993b74f735099dd03f5c6ca`. Their native build identities:

- v43: `resident-v1-b0afadc7bfbd49b5ad10ac04fd51db74051172138256e9d418dfca637b3b6840`
- v50: `resident-v1-7d3956afdd2f5d811a538e41aa30f8d35f832f34791d2df60db86867043523c7`
- v51: `resident-v1-7b8dd766b5d062f124e7aace52d6c052dc156528f73158aae74db8b839fe7ccc`

Whole-binary SHA receipts are joined by replay, not inferred from these labels.
Licenses and attribution are retained in `licenses/`, frozen source and workload
snapshot, including resident MIT/Apache and justrust LICENSE for oracle/workload.

## Standalone offline semantic replay

Only Python standard library and this entire directory are required, any cwd:

```sh
python3 -B /path/to/resident-linear-adjacency-20261031/replay.py
python3 -B /path/to/resident-linear-adjacency-20261031/test_replay.py
```

Replay reconstructs exact input edits/nonces, checks63 source and4020 toolchain
identities, reparses actual raw output and invokes unchanged copies of the five
original semantic-oracle modules. It checks runtime/type/assertion freshness,
exact test names/status/counts/filtered counts, expected exits, no-runtime type
errors and all35 raw rows. Native libtest parsing only normalizes the literal
`- should panic` annotation, preserving test name/status. This is historical
replay, not native re-execution or an independent proof of Rust semantics.

The test driver relocates the whole bundle to an unrelated directory with
spaces, uses isolated Python with empty HOME/PATH, and audits against original
checkout/scratch/toolchain reads and native/network calls. Nineteen negative
controls coherently update changed raw bytes, inner lengths/digests/parsed
results/indexes where applicable and all outer checksums, then require semantic
rejection. They cover nonce freshness, test identity, exit, input recipe, unknown
noise, coherent toolchain mutation, exact native fixture/feature, phase order,
promotion/diagnostic-timing/fresh-phase inventions, snapshot reset/bounds,
failed-analysis preservation and restoration receipt.

`record_checks.py` records offline replay and relocation/mutation results under
`checks/`, then reseals the exact inventory. `SHA256SUMS.bundle` provides
integrity, not provenance authentication or cryptographic signing. Re-sealing
mutations do not model an adversary rewriting the validator itself.
`archive.py` copies existing evidence and needs original paths, but is not
required for replay. No experimental production source is part of the
evidence/doc-only commit. Authored docs and replay code pass scoped diff checks.
`checks/retained-original-whitespace.log` retains the broader diff-check warnings
from byte-identical raw stdout trailing blank lines and archived patch context
lines. Those historical bytes were intentionally not trimmed or rewritten.
