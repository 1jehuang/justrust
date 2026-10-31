# Rejected resident block-scope identity experiment, 2026-10-31

**REJECTED. Production restored. No speed claim, causal attribution, or promotion.**
This is experimental licensed Blaze-derived resident work, not clean-room work.
The bundle preserves actual native journals and 28 real runtime calls, including
failed iterations. The evidence lane did no native job or benchmark rerun.

## Outcome

The candidate moved tracked BlockId creation into one canonical tracked query,
then reused the exact semantic lowering traversal with a private scope-only
output sink for codec block membership. Body and codec shared canonical IDs,
including nested immediate lexical parents and expansion-file AstIds. It kept
the original owner+AstId+table-index wire and def-map schema6. It did not add a
declaration cache. Full implementation and original sources are archived, not
installed or shipped. Production uses the restored original implementation.

Root's actual v49 native gates passed:

- focused `resident_block_scopes`: **12 passed**, 534 filtered, zero ignored,
  including a cold materializing-provider positive control using the same exact
  matcher as the candidate's zero-provider windows
- full hir-def: **545 passed, 0 failed, 1 existing ignored**, no filtering
- original production workspace check and engine build: exit0
- codegen: **53 passed**, resident: **360 passed**, no failures/ignored/filtering
- all **2444** files in each private snapshot copied byte-identically, then only
  Cargo.lock replaced with the previously resolved v24 private lock; source
  manifests after v49 tests unchanged
- original production **3757-file** source manifest and Cargo.lock after checks
  passed. Original workspace manifests and lock were not changed to admit tests.

These validate the tested mechanism, not arbitrary Rust semantics. No assertion
or failing fixture was skipped to obtain the final pass. The one ignored test
is pre-existing `nameres::collector::tests::test_macro_expand_will_stop_2`.

All **28** actual strict runtime calls passed their semantic oracle, but scoped
latency comparisons did not show a useful repeated win. Percentages below are
`100 * (candidate / baseline - 1)`, positive means slower:

| State | First pair | Candidate-first reverse pair |
| --- | ---: | ---: |
| cold-prepare | +48.9135% | -0.8949% |
| repeated-noop-1 | +2.8215% | -28.1059% |
| manifest-feature-add | +16.3825% | +0.9707% |
| type-error | +3.5201% | +1.8361% |
| type-recovery | -0.2584% | +0.2228% |
| assert-failure | +15.2395% | -10.0250% |
| assert-recovery | +2.2654% | -3.4041% |

Noise remains **unknown**, including daemon/background CPU and OS caches.
Waited child CPU is not daemon CPU. First baseline was approximately24 minutes
before candidate, with a build between them. Two counterordered scoped pairs
are not a source-isolated attribution experiment or the comprehensive synthetic
workload. There is no significance claim, general compile speed claim, regression
attribution, or default promotion. Root rejected the candidate despite the
native correctness gates passing.

## Every iteration and failure retained

- `frozen/v46`: original no-op investigation, seven Rust originals unchanged in
  its candidate, doc/patch/hashes/readiness. This is not the implementation.
- `frozen/v47`: full source-only implementation, eight authorized Rust originals
  and complete candidate plus doc, exact patch/hashes/source checks/release.
- `native/candidate-build-v47`: actual original-workspace check exit101,
  **103 errors**: excluded-path hir-def test target could not resolve existing
  devdeps `expect_test`, `test_fixture`, `test_utils`, causing cascading errors.
  Root did not repair fixtures to hide a workspace admission failure.
- `snapshots/rust-analyzer-v48.tar.gz`, `native/snapshot-v48`: entire private
  rust-analyzer source, actual focused **8 passed/4 failed**, full **540 passed/
  5 failed/1 ignored**. Three focused assertions used wrong exact Salsa event
  names, the positive lang-item fixture omitted feature(lang_items), and the
  fifth full-suite failure was only internal BlockId number881 versus901.
- `frozen/v49`: separate full originals/candidates, rejected live six-file
  preservation, patches, hashes, source checks, explicit release, both v48
  failure logs. Production same as v47. Fixture fixes use exact underscore
  event names and add the cold nonvacuous Body observer, enable the required
  feature and require independent original-fullwalk membership3 before candidate
  comparison. The sole existing snapshot update is observed numeric881->901.
- `snapshots/rust-analyzer-v49.tar.gz`, `native/snapshot-v49`: entire actual
  source and final focused/full native passes, not copied success summaries.
  Raw justrust output retains exact12 fixture names, statuses and totals.
- `native/candidate-build-v49`, `native/candidate-unit-v49`: original production
  check/build, engine identity, raw53/360 suite journals, all-source3757 and
  production-lock before/after. `native/candidate-{build,unit}-v43` retains
  accepted baseline native provenance and engine identity.

All archived native run directories retain original meta.json, output.jsonl,
summary.json and units.jsonl when present. Original displays, errors, exit files,
source manifests and launcher scripts are retained without trimming. Runtime
worker logs are cumulative per state, not independent per-phase daemon samples.
No cleanup, store deletion, or native rerun was performed by this evidence lane.

### Native run identities

- failed excluded workspace check: `20261030-213148772-1992017` (the actual
  retained native/candidate-build-v47 run directories are authoritative)
- v48 focused: `20261030-213513023-2010200`
- v48 full exploratory: `20261030-213759044-2027453`
- v49 focused: `20261030-214133450-2047116`
- v49 full hir-def: `20261030-214206957-2051649`
- v49 codegen: `20261030-214528212-2077166`
- v49 resident: `20261030-214646807-2086833`

Candidate version reports Blaze-derived origin
`5d4f78f7577cf7170993b74f735099dd03f5c6ca`, build
`resident-v1-2f6aac62db11a4784f0c3e4575150b62cef2874e991734f333e535ee3c02aa86`.
Accepted v43 build:
`resident-v1-b0afadc7bfbd49b5ad10ac04fd51db74051172138256e9d418dfca637b3b6840`.
Engine digests in native build receipts join every raw runtime state before/after.
Pinned toolchain is nightly-2026-10-27-x86_64-unknown-linux-gnu, complete4020-file
identity before/after per state, including channel-manifest SHA
`34f3d9bf04640a082e561310f3f258846397f4cf64a2bcfebcea96ebf18e2572`.
Binaries, private caches and toolchain artifacts are not embedded.

## Runtime source, private snapshot and restoration

`raw/{block-scopes-v47-baseline,block-scopes-v49-candidate,
block-scopes-v49-candidate-repeat,block-scopes-v49-baseline-repeat}` retains every
original state.json, stdout/stderr, worker-log snapshot and three original input
backups. `rows28.json` is an exact index, not new measurements. Replay reconstructs
all original edits from those backups and invocation nonces, recomputes hashes,
parses raw output and runs the unchanged original semantic oracle. It checks
runtime/type/assertion nonce challenges, exact test identity/counts/filtered
counts, expected success/failure exits and no-runtime type errors.

`snapshots/source63.tar.gz` contains the pinned63 workload source bytes. Both
rust-analyzer snapshots contain all2444 source files, Cargo manifests/lock and
licenses. Original license symlinks are preserved and resolved only within the
archive during replay, never extracted or followed to the host. The snapshots'
sibling source links were the six exact native vendored directories listed in
the preserved prepare scripts, not rewritten Rust. Complete original/selected
manifests and v49 after joins are retained. `provenance/original-vendor-Cargo.lock`,
`private-snapshot-Cargo.lock` and `private-lock.patch` show the exact private
resolution delta. Production Cargo.lock is separately retained and unchanged.
The copy's Cargo.lock substitution is deliberate unit-test workspace admission,
not byte-identical dependency resolution with the production workspace.

`restoration.json` records actual cmp and HEAD checks of all six restored Rust
files at 2026-10-31T04:54:38Z, after preserving rejected live bytes. Ownership was
released at04:54:45Z. Base HEAD at restoration:
`ed7ba926b5b5c1a8adc024c1d2d0b72e385dd00a`.
No resets/checkouts, broad snapshot updates or unrelated edits were used.

Licenses and original attribution are in each source snapshot and `licenses/`,
including justrust's LICENSE for the pinned workload and copied oracle modules,
and original resident MIT/Apache texts and upstream README. Source3757 and
TC4020 are complete identity manifests, not full binary/toolchain distribution.

## Standalone offline replay and semantic falsification

Only Python standard library and this entire directory are needed, from any cwd:

```sh
python3 -B /path/to/resident-block-scopes-20261031/replay.py
python3 -B /path/to/resident-block-scopes-20261031/test_replay.py
```

The driver relocates this whole bundle to an unrelated directory with spaces,
uses isolated Python and empty HOME/PATH, and installs an audit hook forbidding
original checkout/scratch/toolchain reads plus native subprocess/network calls
inside replay. Replay needs no installed engine, compiler, source checkout, git,
third-party Python package or private lock resolution. Historical absolute paths
remain unchanged in raw evidence but are never dereferenced.

Fourteen semantic mutations update affected inner raw digest/length, parsed
results, row indexes and all outer checksums before replay: runtime/type/assertion
freshness, missing exact runtime test, success/failure exits, input recipe,
unknown-noise classification, coherently changed toolchain receipts, exact native
fixture identity, phase order, additional private source delta, invented promotion
and restoration receipt. They must be rejected for semantic reasons after their
transport hashes pass. This is coherent checksum re-sealing, not cryptographic
signing or protection against an adversary rewriting the validator and evidence.

`oracle/` contains unchanged copies of the five original harness modules used by
the actual probe. Replay invokes their semantic contract on actual raw output,
not current repository modules or recorded accepted flags alone. For native
libtest output only, it normalizes the exact `- should panic` annotation while
preserving full test name/status for strict parsing. This is historical evidence
replay, not native re-execution or an independent proof of language semantics.

`archive.py` copies existing journals and source only and needs original local
paths; it is provenance, not required for replay. `record_checks.py` generates
explicit lock-delta artifacts and runs only offline checks, preserving reports
under `checks/` before resealing. `SHA256SUMS.bundle` checks every retained file
and inventory. It provides integrity, not provenance authentication/signatures.
No experimental production change is part of the evidence-only commit.
