# Native home-query v66/v67 evidence, October 31, 2026

**Decision: rejected, source restored, no promotion.** Root's actual repaired-v67
native correctness gates passed, but the two same-binary OFF/ON pairs showed no
repeatable useful scoped latency improvement. Noise is unknown. No speedup,
causal regression, general/default, or actual workload cache-hit claim is made.
See `../../../docs/resident-home-query.md` from this directory's parent layout,
or repository document `docs/resident-home-query.md`, for the complete narrative.

## What is preserved

- `observations.json`: fixed-field derivatives of all four seven-phase runtime
  cases, including original allowed stdout, fresh nonce/type/assertion witness
  lines, exact parsed test contracts, exit status, safe controls, status command
  outcomes, cumulative provider-entry snapshots, and exact original raw-file
  SHA/byte provenance. Every original state/raw/worker artifact used is bound
  in `private_originals`; raw private state and logs are not published.
- `inventories/source63.json` and `toolchain4020.json`: full relative workload
  source and pinned-toolchain inventories, without the private toolchain root.
  All four cases' before/after identities agree. Row input inventories retain
  the controlled per-phase source changes separately.
- `inventories/v66-native-source3757.json` and `native-source3757.json`: complete
  failed-v66 and passing-v67 gate source hashes, bound to their different home
  implementations. Source/lock unchanged gate results are also retained.
- `native/codegen-results.json` and `resident-results.json`: every actual named
  native test result, 58 and 364 respectively, plus original summary lines.
  All five `home::tests::native_home_*` fixture names are checked explicitly.
- `frozen/baseline/`, `v66/`, `v67/`, and `live-before-restoration/`: complete
  relevant Rust/manifest snapshots, distinct proposal-document derivatives,
  original freeze records, original failed-v66 patch derivative, v67 incremental
  patch derivative, and exact-context per-file v66-to-v67 repair patches.
  Rust and manifest files are exact originals. Documents/scripts containing
  private path constants are **sanitized derivatives**, not claimed byte-exact
  private originals. Provenance retains hashes and sizes of the originals.
- `frozen/home-query-v66-root-integration--lock.patch.txt`: root's only two
  already-resolved fixture edges, `ide-db` and `test-fixture`. No new package
  resolution. The source lane did not edit the lock.
- `analysis/`: root's original successful rejection analysis output and a
  sanitized derivative of the exact historical analysis source.
- `oracles/`: historical strict oracle sources, actual runtime-driver source,
  and native-gate script as non-executable text derivatives. Replay does not
  import or execute their build/daemon/process code.
- `checks/`: exact source restoration and pre-restoration-live hash receipts.
  Restoration is a point-in-time receipt, not a claim about subsequent work by
  other source-owning lanes.

V66's actual gate failed 101 with `E0277` native-key trait requirements and
`E0271` slice iterator item mismatch. V67 uses the supported native tuple
interner `(full Instance, constant unit)` and explicit borrowed slice iteration.
Its strict native fixture source is bytewise unchanged. Failed-v66 error
headers and original complete-log provenance are preserved, not erased by v67.
The bundle-local `.gitattributes` preserves required unified-diff blank-context
spaces only for frozen patch artifacts. It does not change source or replay
code whitespace checks.

The five real `RootDatabase` fixtures established cold-one/repeated-zero native
provider execution capability, direct-oracle equality, signature/dependency and
closure-owner key changes, equal-output recomputation, an irrelevant body-only
revision control, and opaque fallback. That does **not** demonstrate measured
workload cache reuse. OFF snapshots contained zero marker lines. ON snapshots
contained one **cumulative once-per-process QUERY ENTERED** marker each. Later
snapshots repeat the same earlier line, not additional execution/hit evidence.

## Offline replay

From any working directory, using an absolute path if the bundle is relocated:

```sh
python3 -B validate.py --manifest
python3 -B test_offline.py
```

Replay uses only Python's standard library and packaged files. It never invokes
Rust, Cargo, justrust, the resident engine, a toolchain, a daemon, Git, or network
services. It reconstructs the 28 strict outcomes, named/count/status and fresh
runtime/error witnesses, source/toolchain identities, actual full native result
sets, five fixtures, root pair arithmetic, source repair, and restoration hashes.
`SHA256.json` covers the complete bundle. A fixed digest in `validate.py` binds
selected historical observations independently of that mutable inventory. This
is a historical commitment, not an external authenticity/signature guarantee.

Observed offline controls: seven Python tests passed, with 40 semantic negative
mutations, a coherent wrong-test raw/parsed pair, a rehashed failed-native-result
mutation, a coherent timing/root-output/arithmetic rewrite, exact patch-context
rejection, privacy selection checks, and unrelated-directory relocation with
empty HOME, empty PATH, and Python `-I -B`. These are evidence checks only, not
additional native acceptance or performance tests.

## Privacy and nonclaims

`export_private.py` is the fixed allowlist export recipe. Regeneration requires
existing private artifacts and intentionally is not part of portable replay.
It reads only named artifact paths, selects eight non-path environment values
and five explicit controls, and admits only known stdout/diagnostic/marker line
grammars. It does not dump or redact the ambient environment. Twelve private
path-bearing entries per runtime receipt are omitted, not republished. Full
original artifact SHA/size provenance binds every selected input. Private
roots in source/script constants are replaced by labeled placeholders only in
those selected historical text derivatives.

Excluded: ambient environment, raw private state JSON, private homes and caches,
raw worker/daemon logs, telemetry, credentials, executables, build targets, and
unrelated files. Toolchain paths are relative inventory names, not installed
private locations. The fixed historical parser/semantics are not weakened to
accept future or synthetic results. No native command is rerun to fabricate
missing history. The candidate is not shipped by this evidence-only commit.
