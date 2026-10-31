# Rejected native declared graph query v69 evidence

**Rejected for performance, original source restored, no promotion.** All 28
actual strict runtime rows and the 53/367 full native gates passed. Two paired
OFF/ON seven-phase loops did not show a useful replicated scoped latency win.
Noise is unknown. This bundle claims neither a speedup nor a causal regression.
See `docs/resident-declared-query.md` for hypothesis, implementation, dependency
and cycle audit, exact timing table, marker limitations and restoration.

## Retained evidence

* `observations.json`: fixed-field derivatives of four actual runtime cases,
  all selected original stdout, exact named-test contracts and process statuses,
  fresh runtime/type/assertion witnesses, allowed controls, PID provider-entry
  snapshots, elapsed times, and SHA/byte provenance of private originals.
* `inventories/source63.json`, `toolchain4020.json`, and
  `native-source3757.json`: full relative file inventories. Workload/pinned
  toolchain identities agree before/after all cases. Each runtime row retains
  its input inventory. The 3757-source native build snapshot includes the exact
  candidate main hash. The native gate checked that snapshot and Cargo.lock
  unchanged after testing; there were no lock edits for this candidate.
* `native/codegen-results.json` and `resident-results.json`: all 53 and 367
  actual named passing lines and full original summaries. Resident results
  include the three actual `declared_query_` fixtures, including two real
  RootDatabase differential/revision fixtures and the pure exact flag fixture.
  Native run IDs are preserved in observations. These are actual gate results,
  not a synthetic replay of the Rust implementation.
* `frozen/baseline/`, `candidate/`, `live-before-restoration/`: byte-exact full
  Rust source snapshots. Candidate/live proposal documents and the original
  complete patch/release/sourcecheck receipts are sanitized selected text
  derivatives. Raw original SHA/size provenance is retained separately.
  `change-main.rs.patch` replays the exact Rust source change with full context.
  The original disabled algorithm is bytewise unchanged in the candidate.
  Bundle-local `.gitattributes` exempts only the two historical unified-diff
  files from blank-at-EOL checks so single-space context lines remain exact.
  Full Rust snapshots and every other artifact retain normal whitespace checks.
* `analysis/root-output.json` and `root-analysis.py.txt`: root's actual
  successful rejection analysis and a sanitized derivative of its historical
  strict raw/state reanalysis source. Two descriptive pair comparisons retain
  original seconds and recomputable arithmetic.
* `oracles/`: historical original pure parser/oracle sources, actual runtime
  driver and actual native gate script as text. Replay extracts only original
  pure functions by AST and never imports execution drivers.
* `checks/restoration.json`: worker's actual live-to-frozen comparisons,
  preserved snapshots, write-only main restoration, exact original comparison
  and scoped empty HEAD diff at `2026-10-31T12:50:57Z`. This is a point-in-time
  receipt. Source was then released to the coordinator for separate work.
* `observations.json` also retains safe fields of root's actual four completed
  namespace stop receipts. Stops were post-measurement, exited 0, and preserved
  stores. Raw private paths and environment were not exported.

The provider marker is cumulative once per process. OFF snapshots contain zero
lines in all seven phases. Both ON snapshots grow as `[1, 1, 2, 3, 4, 4, 4]`.
They prove actual enabled provider entry only, **not cache hits**, successful
completion, per-phase query counts, avoided lowering, or latency attribution.
Worker replacement can add a new process entry. Actual-provider-only test TLS
counters, not these runtime marker snapshots, enforce fixture cold1/repeat0.

## Portable offline replay

From any working directory, including a relocated bundle:

```sh
python3 -B /path/to/bundle/validate.py --manifest
python3 -B /path/to/bundle/test_offline.py
```

Only Python's standard library and packaged files are used. No Rust, Cargo,
justrust, engine, daemon, toolchain, Git or network job runs. All 28 rows are
freshly parsed from selected actual raw text and checked by the original
hash-bound pure contract/oracle functions. The original tuple equality is not
weakened to accommodate JSON lists. An independent exact named/count/status
oracle adds separate checks. Full native named sets, inventories, candidate
patch replay, bytewise original algorithm, pair arithmetic, rejection and
restoration are also verified.

`test_offline.py` contains eight tests, including 37 semantic negative mutations,
coherently wrong raw-plus-parsed test names, exact patch-context rejection,
original tuple/list behavior, rehashed failed-native-result rejection and a
coherent elapsed-time plus matching root-arithmetic rewrite. The latter can
pass arithmetic semantics but cannot rewrite the fixed observation commitment.
The relocation control uses an unrelated directory, empty HOME, empty PATH and
Python `-I -B`, and requires HOME to remain untouched. These checks verify
historical evidence, not additional native acceptance or performance.

`SHA256.json` inventories the complete package. Independent fixed digests in
`validate.py` bind historical observations, full native selected journals,
full Rust snapshots and pure original oracle sources. They are historical
integrity commitments, not an external signature/authenticity guarantee.

## Fixed privacy allowlists

`export_private.py` is an explicit allowlist recipe requiring existing private
artifacts only for regeneration. Portable replay does not need those artifacts.
It selects eight non-path environment values and five startup controls, never
ambient environment. Twelve path-bearing environment fields per row are omitted.
Full stdout must match the exact test/witness/summary line grammar or export
aborts. Only exact E0308 headers, typed nonce lines and assertion witness lines
are selected from stderr. Provider marker snapshots must match the exact
PID-entry grammar and prefixes of the retained final marker journal. Full raw
private state/worker logs are not published, only hash/size provenance.

Selected path-bearing source/script/document constants are replaced by labeled
placeholders. Approved byte-exact full Rust snapshots retain existing synthetic
fixture literals such as `/home/one/.cargo`; they do not expose the user's home.
Excluded: raw private state and worker/daemon logs, ambient environment, private
cache/home roots, credentials, telemetry, executables, targets and unrelated
files. Export checks that every consumed private original is unchanged afterward.
The source-restoration/evidence worker ran no native jobs or installs.
