# Rejected resident typed-planning candidate, October 30, 2026

**Decision: REJECTED / NOT PROMOTED.** Correctness gates passed after fixing the original invalid library fixture, but repeated native timing did not establish reliable total benefit. Both live production files were restored byte-for-byte to HEAD, and the uncommitted live test was removed only after its exact source was preserved here. This directory is evidence, not an enabled optimization or clean-room implementation. Source is licensed Blaze-derived experimental resident code.

## Reproduce the offline audit

From any working directory, with Python 3.9+ and only its standard library:

```sh
python3 /path/to/resident-typed-planning-20261030/validate.py
python3 /path/to/resident-typed-planning-20261030/test_validate.py
```

The validator accepts an optional bundle path. It never executes native code, accesses the network, imports repository benchmark modules, or opens original absolute paths in receipts. Tests copy the bundle to a temporary directory, validate from an unrelated working directory, and exercise rehashed contradictions. Set `JCODE_SCRATCH_DIR` to choose test-copy scratch space. No Rust build is needed.

`validation.json` is derived validator output. Raw artifacts are sealed by `manifest.json`. `make_manifest.py` is the explicit archival sealing utility, not a validator fix-up step. Do not reseal modified evidence to make a failed audit pass. Validators, tests, README and derived validation output are intentionally outside the raw hash manifest. This is consistency/provenance validation, not a signature, a fresh runtime test or proof of causal speed improvement.

## Total performance, all retained observations

| Suite | Engine median seconds | Matched reference median seconds |
|---|---:|---:|
| RSA01, v4 baseline | 0.10961705099907704 | 0.1109231570007978 |
| RSA04, v8 candidate | 0.10225796500162687 | 0.11854641500394791 |
| RSA05, v8 repeat | 0.1134313939983258 | 0.1157766399992397 |

Each suite contains five actual edited runs per stack plus preparation, deliberate type-error and assertion-failure cases, and recoveries. All 22 observations per suite, raw stdout/stderr, metadata and hashes are retained. Against RSA01, RSA05 engine elapsed is **3.4797% slower raw**. Reference normalization, `(candidate_engine/candidate_reference)/(baseline_engine/baseline_reference)-1`, is **-0.8583%**, below the 10% comparison floor. RSA04 is retained, not cherry-picked as a successful optimization. These are scoped diagnostic native workloads with unknown noise and rustc-built proc-macro compatibility control, not the comprehensive session-weighted gate or a default-promotion result.

## Matched phase evidence

`profile/` retains both warmups and all ten alternating actual edited observations, all twelve full native traces and stdout/stderr, original source, source restoration and tool identity receipts. The stack order is old/new for even edits and new/old for odd edits. Medians exclude the two warmups:

| Nested elapsed phase, microseconds | Old v4 | New v8 |
|---|---:|---:|
| Client planning | 21160 | 20599 |
| Client metadata | 529 | 523 |
| Client command | 210462 | 105761 |

The client-command intervals vary strongly, and include differing retained daemon/cache state, including the very slow old warmup. Neither these medians nor subtracting nested phases demonstrates causal savings. Phase intervals are not additive CPU costs. The smaller planning difference did not establish reliable total workload benefit in RSA05.

## Retained evidence inventory

- `candidate.patch` and `candidate/`: full exact modified library/caller and the repaired five-test differential fixture. The portable validator reconstructs all three files from the patch and archived baselines, then checks pinned SHA-256 values against native build source receipts.
- `baseline/`: exact two-file HEAD backup captured at `23a1490cb84fa0aaca872dcf6a430f35e19a3863`. Only these two live files were restored. Other workers' changes were not touched.
- `build-v7/`, `unit-v7/`: original native check success followed by the four-test failure caused by illegal simultaneous `dylib` and `cdylib`. It remains a failure and admitted no performance samples.
- `build-v8/`, `unit-v8/`: actual native check/build, five typed tests, 49 existing metadata/manifest tests (2 manifest, 47 resolve, 0 metadata lib tests), one graph test, one listed-roots test and three telemetry tests. Raw recorder text/JSON runs preserve full output, not just summary counts.
- `recorder-exports/`: nine root-exported slim runs, four v8 build-side runs, three v8 unit-side runs, and two rejected v7 runs. Validator binds IDs and verdicts/test counts to raw receipts, accounting for intentional binary-name redaction and millisecond rounding. No global `data/runs/` files were changed by this lane.
- `rsa01/`, `rsa04/`, `rsa05/`: complete top-level suite receipts and raw outputs, without private caches, native binaries or dependency/build trees.
- `parity-failed-dispatch/`: original incorrect direct `metadata` invocation, two failed process observations and failed state. It is not successful parity evidence.
- `parity-six-cases/`: corrected actual `jr-engine cargo metadata` invocation, six old/new pairs comparing **exact status, stdout bytes and stderr bytes**. Cases are full metadata, no-deps, all-features, no-default-features, missing-feature rejection and bad-format rejection. Fixture manifests/source/lock file are retained. These cases do not cover every possible flag/manifest.
- `scripts/`: exact root gate/profile/parity/suite scripts and both historical source-ready receipts. Scripts contain original local absolute paths and are provenance, not automatically portable workload launchers. The corrected metadata script explicitly includes the `cargo` dispatch.

Full original paths inside raw evidence are preserved for fidelity. There are no native executables, shim symlinks, caches or databases in this bundle. Source/tool hashes identify measured artifacts but the executables themselves are intentionally not shipped.

## Audit's scope

The validator checks complete local artifact inventory/hashes, exact candidate and baseline source, patch reconstruction, built-source identity, expected v7 failure and v8 native gate verdicts, all nine recorder IDs, all three suites' runtime/failure/recovery outputs and recomputed medians, six actual metadata parity pairs, and all twelve matched phase observations against raw trace events. Twenty-three audit tests cover portability and mutations such as rehashed source/patch changes, nonzero gates, relabeled failed attempts, incomplete parity/trace inventories, changed dispatch/status/output bytes, absent fresh-runtime markers, recorder-ID/count changes, median tampering, binaries and symlinks. These are offline evidence tests, not replacement native workload tests.

The experiment rejected only this bounded final JSON roundtrip removal as a demonstrated total-speed optimization on the measured workload. It does not prove that all typed planning, metadata design or resolver changes are unhelpful. A future proposal needs a new measured hypothesis, exact semantic gates and matched repeated workload evidence.
