# Resident cache-miss attribution, 2026-10-31

Actual default-off native diagnostic evidence for source commit `cb9d8a010`.
This is not a performance improvement, unique-function loss accounting, or a
causal explanation of the manifest/recovery compilation gap. No timing comparison
is made. All counter populations overlap and can include repeated observations.

## Native and workload gates

* v42 actual check/build passed. Its native codegen gate failed **50 passed,
  3 failed** in `20261030-201323863-1411935`. The exact failed assertions and
  original sources remain retained, not replaced by the repaired result.
* Actual native store semantics explain the fixture defect: insert appends an
  open pack, while get sees sealed packs. v43 adds only real `Store::seal()` and
  exact byte readback to the fixture helper. All expected counters and production
  branches stay unchanged. `mod.rs` is identical between v42 and v43.
* v43 actual check/build passed. Codegen **53/53** passed in
  `20261030-201905159-1445554`; full resident **360/360** passed in
  `20261030-201926060-1448168`. The 3,757-source and lock receipts are retained.
* Both enabled and disabled actual seven-state sessions passed. Each covers cold
  preparation, repeated noop, manifest feature addition, expected type error,
  type recovery, expected assertion failure, and assertion recovery. Expected
  failures retain exit 101. Source63, 4,020 toolchain file identities, native
  executable identity, invocation-specific input recipes and restoration bind
  these rows. No engine executable or toolchain binary is retained.
* `gates/v43-bench-regression.log` is the actual **733-test PASS** log.

The enabled manifest snapshot has all 19 actual fields, including direct missing
45,508, arrived 45,207, native imports 9, unavailable dependencies 25, settled
negative dependencies 1 and transitive failures 26. Type recovery records direct
missing 34,863, arrived 55,754, native imports 9, unavailable dependencies 26,
settled negative dependencies 3 and transitive failures 29. Decode/soundness,
debug-type, remapping and banned counters are zero in these observations.
**Do not sum these as unique lost functions.** The same last type-recovery
snapshot appears unchanged in later assertion/patch/noop cumulative logs. It is
not a fresh assertion-phase or per-patch observation. Disabled stdout, stderr
and worker snapshots contain zero native report lines.

## Retained material

* `frozen/v42` and `frozen/v43`: complete originals, candidates, patches, original
  SHA receipts and readiness/release notes. `repair-from-v42.patch` is replayed
  independently. Original frozen source/doc bytes are unchanged.
* `frozen/store-lib-cb9d8a010.rs`: immutable committed native store source,
  retrieved with `git show cb9d8a010:...`, not from the live Rust checkout. Its
  hash joins both actual 3,757-file manifests. This retains the real publication
  contract and native store fixture, not a simulated implementation.
* `gates`: raw check/build/unit logs and exits, source/lock checks, engine
  version/digest receipts, and actual 733-test regression output.
* `journals`: complete original meta/output/summary records for seven native
  runs, including every passing/failing test name and the failed v42 assertions.
* `slim`: derived slim summaries joined back to raw native journals.
* `runtime/on` and `runtime/off`: all actual stdout/stderr/worker snapshots,
  original state records and durable source backups, 14 rows total.
* `input/source63`, `licenses`: complete pinned workload inputs with their
  original licenses and native Blaze MIT/Apache attribution.
* `provenance`: original runners, root validation script and pure parser/oracle
  helpers. Historical absolute paths remain provenance strings, never audit
  dependencies. `root-validation.json` is the exact original root result.
* `commitments.json`: independently pinned frozen source/helper/native/workload
  identities and all 19 actual report values for every snapshot.

Only existing frozen sources and receipts were read for collection. No live Rust
reads/edits, native runs, cache/database files, ELF files, telemetry homes,
credentials or full process environments are included. Retained environment
fields are restricted to explicit nonsecret allowlists, not ambient recapture.
The archived runner scripts are historical native inputs, not audit commands.

## Portable offline audit

With Python 3 and its standard library, from any relocated copy:

```sh
python3 -B validate.py
python3 -B test_validate.py
```

Neither audit starts a native engine, compiler, benchmark, Git process or network
connection. The validator checks complete transport inventory, independent
historical commitments, full native patch replay, exact fixture-only repair,
3,757-source receipts, native exits and complete test-name populations, slim
joins, actual strict state oracles and per-invocation challenges, restoration,
and every actual native report field. A repeated snapshot is explicitly not a
fresh assertion observation. `validation.json` is derived, not a new native run.

The author controls relocate the package, clear HOME/PATH, forbid original
checkout/home/scratch file reads and process/socket operations, then reject
**25 coherently re-sealed semantic corruptions**. Controls include startup
flags, missing state, source/tool/toolchain/input/restoration identities, actual
test names, environment allowlist, unsupported clean-noise claims, status/time,
invocation/error/assertion markers with nested receipts regenerated, malformed
or private report fields, altered actual counts plus coherently updated root
JSON, disabled reports in worker and stdout, native fixture coverage, changed
733-test result, and falsely fresh assertion observations. These mutate only a
relocated temporary copy, not the retained originals.

`SHA256SUMS` is a re-signable transport checksum inventory, not a cryptographic
authenticity signature. The separately pinned commitments prevent changing
historical identities or observations just by recomputing this inventory.
`validation.json` is excluded because it is derived. `audits/` retains the
actual author audit output. Passing this offline audit does not rerun or replace
native correctness tests and establishes no speedup.
