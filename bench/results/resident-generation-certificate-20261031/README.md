# Native generation certificate: rejected experiment evidence

Date: 2026-10-31. **Evidence only, not a production implementation or promotion.**

All nine candidate Rust files were restored to accepted base `69669f0c7b28721fc3130eb801311e7e7afca962`. This package records what actually happened, including unsuccessful source freezes and an expensive positive retention case. The historical explanation is `toolchain/resident/docs/resident-generation-certificate.md`.

## Portable offline checks

From any working directory, using only Python's standard library:

```sh
python3 /path/to/this/package/validate.py --manifest
python3 /path/to/this/package/test_offline.py
```

These commands **do not invoke Cargo, rustc, justrust or the resident engine**. The validator accepts only exact expected outcomes, not a weaker reported pass count. The tests exercise actual selected observations, 28 adversarial mutations, export privacy and validation after relocating the package into an unrelated directory. A modified observation fails the package checksum independently of semantic checks.

`SHA256SUMS` covers the five package files other than itself. Its hashes are integrity checks, not cryptographic third-party attestation. The neighboring historical document is outside this relocation unit and is not included in this manifest.

## Contents and provenance

- `observations.json`: explicit-allowlist derivatives of coordinator-owned actual runs. Includes 35 correctness rows, compiler iteration outcomes, native unit summaries, candidate source inventories, accepted-source restoration digests and post-measurement process-stop receipts.
- `export_private.py`: offline derivative generator. With access to the original private experiment directory, run `python3 export_private.py PRIVATE_EXPERIMENT_DIRECTORY --output observations.json`. It reads a fixed set of files and selects fixed schema fields and full-line diagnostic patterns. It never runs a compiler or executable. Do not point it at unrelated data.
- `validate.py`: independent exact semantic oracles for the selected observations. An original harness `accepted=true` is necessary but never sufficient.
- `test_offline.py`: semantic mutations, metadata-drop tests and actual package relocation validation.

Raw state/log hashes are retained so a custodian can check private originals. Private raw files are **not** shipped, reconstructed or required by the portable validator. Observations are deliberately reduced and do not replace the full original diagnostic record. Original environment maps, command environments, toolchain path listings, private cache locations and raw path-bearing logs remain private and unmodified. No broad redaction pass over raw environment dumps is used. Nonces and PIDs in this package are synthetic request witnesses and observed process IDs, not environmental identities.

The root coordinator ran all native gates. The source/evidence worker performed source formatting, exact source restoration and offline evidence validation only. The recorded restoration time is `2026-10-31T10:18:22Z`. All nine immutable-original comparisons and scoped HEAD diffs passed before packaging. Completed OFF/ON v64 processes were stopped afterward by the root without deleting stores.

## Actual oracles, not softened counts

### Same binary, fourteen seven-state rows

OFF and ON each have exactly these ordered phases: cold prepare, repeated no-op, manifest feature add, type error, type recovery, assertion failure, assertion recovery. The same executable digest is required in both groups.

Every successful test row must name exactly these four tests, in order:

1. `summary::tests::parses_libtest_results`
2. `summary::tests::shares_overlapping_time`
3. `summary::tests::splits_passes_without_double_counting`
4. `summary::tests::test_failures_are_not_compile_errors`

Success requires all four `ok`, status 0, a unique fresh runtime nonce, and exactly `4 passed; 0 failed; 0 ignored; 0 measured; 144 filtered out`. Assertion failure requires the second named test alone to be `FAILED`, the exact 3/1/0/0/144 tuple, status 101 and its fresh assertion diagnostic literal. Type error requires status 101 and its fresh `E0308` source literal with no completed test summary. Timeouts, transport errors, replayed nonces, missing rows and changed counters fail validation.

### Completed retention and actual fallback

ON manifest feature addition has unchanged actual worker status PID **151379**, a changed-generation parent observation and exactly one newly completed retained-load marker for that PID. Cumulative daemon snapshots are compared by prefix, so old markers cannot masquerade as a new completion. An eligibility line alone fails the oracle.

The separate manifest fixture has exactly nine original/OFF/ON × valid/error/recovery rows. Successful programs must print their exact fresh `GEN_OK` nonce. Invalid configuration must preserve status 101 and the fresh source-error literal, with no program execution. The ON error request has exactly one new retained-load failure/cold-restart event and exactly one new cold worker. Recovery has a different PID and a fresh successful nonce. This proves that observed fallback request, not universal fallback safety.

The A/B/A native archive fixture must print exactly fresh-nonce values 11/22/11. v62 OFF, v62 ON, v63 ON and v64 ON are retained separately. v63's three client `unknown stage=plan` results remain visible. v64's archive selection requests have the same worker PID but the same source generation, so they are not counted as the changed-generation completion proof.

### Compiler and unit outcomes

| Version | Actual compiler check/build | Actual native unit gates |
|---|---|---|
| v60 | check 101, two E0599 errors, missing `anyhow::Context` import | not run |
| v61 | check 101, E0282 ambiguous `AsRef` conversion | not run |
| v62 | check/build 0 | 53 codegen, 374 resident, all passed |
| v63 | check/build 0 | 53 codegen, 374 resident, all passed |
| v64 | check/build 0 | 53 codegen, 375 resident, all passed |

The v64 resident count increased by one for the strict client/worker selection parity fixture. No assertion or historical test count was reduced. Unit receipts in this package preserve coordinator output summaries and their raw-log hashes, not every individual unit-test line. They are historical outcomes, not new offline native runs.

## Rejection and timing limits

One completed retained load was achieved, but the candidate was **rejected**. Full effective native ABI/config safety remained unproved, including the environment-only root-config merge gap. This package does not claim those unrun negative/native boundaries passed.

Single observed wall seconds, same v64 executable:

| Phase | OFF | ON |
|---|---:|---:|
| cold prepare | 30.496294 | 37.965017 |
| repeated no-op | 0.038374 | 7.041847 |
| manifest feature add | 8.886862 | 20.334796 |
| type error | 9.262103 | 20.495901 |
| type recovery | 11.071772 | 23.860282 |
| assertion failure | 2.895534 | 5.904080 |
| assertion recovery | 0.581097 | 4.929700 |

These are **descriptive uncontrolled observations**, not significance estimates, a comprehensive benchmark, or a generic regression claim. Original noise classification was unknown, substantial other CPU use was observed, daemon CPU was not waited-child CPU, and the executions were sequential, not randomized. They establish no useful measured win for this candidate. There is no speedup or default promotion claim. A future attempt requires new source identity, complete correctness boundaries and the repository's comprehensive measured gate.
