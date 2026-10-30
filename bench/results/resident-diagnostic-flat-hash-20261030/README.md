# Flat diagnostic hash, October 30, 2026

Portable offline evidence for source commit
`bea4cc1dc29dbe65b1c2073c92e43f79e4d735a2` (shipping `stable` repository,
experimental resident engine path). No native command is run by the validator,
sealer or mutation tests. Original runners are provenance, not portable replay
commands and must not be executed as part of this audit.

## Replay

From any checkout or standalone copy containing this directory, with Python 3.9+
(the validator uses `Path.is_relative_to`):

```sh
PYTHONDONTWRITEBYTECODE=1 python3 validate.py
PYTHONDONTWRITEBYTECODE=1 python3 test_validate.py
```

Use the actual path to those scripts when running outside this directory.
`validate.py --root /path/to/copied/bundle` also works. The audit never resolves
original source, toolchain, binary or daemon paths. Those strings remain in raw
records as historical identity/provenance. The portability test replaces all
collection origin paths with unavailable paths and replays successfully.
Mutation tests use `JCODE_SCRATCH_DIR` when set, otherwise the platform temporary
directory. They remove their own temporary copies.

`manifest.json` seals the complete inventory except itself and `__pycache__`.
`collection.json` records 860 original artifacts with byte-verified copies and
original SHA256 values. `audit.json` is derived output, not an input trusted for
native counts, test names, markers, trace events, callback intervals or medians.
`seal.py` regenerates the derived audit/inventory after intentional bundle edits.
Do not reseal to conceal a changed raw artifact. `collect.py` is an archival
one-shot collector tied to original paths, not required for portable validation.

## Accepted scope and non-claims

This is a scoped experimental warm selected-fixture improvement. It is **not**
a comprehensive session-weighted benchmark, cold-build result, significance
claim, general build-speed claim, or default promotion. Resident daemon CPU
noise was not established to be absent. All sample/order observations remain.
The hash implementation borrows flat backing text, while green syntax retains
the exact allocated-string fallback. The two differential tests and all native
gates passed, without adding production counters.

| Evidence | Raw validation |
| --- | --- |
| `journals/v23/` | Each focused test's actual passing names and libtest result, full 351 unique passing names, zero failed/ignored, exit 0 |
| `journals/v22-restored/` | Corrected baseline full 349 unique passing names and result, exit 0 |
| `native/` | Focused admission receipts, human-readable reports, selected-source boundary logs |
| `provenance/bench-regression-v23.log` | Actual `Ran 700 tests` and final `OK`, exit receipt 0 |
| `fixed-five/{rsb14,rsa20}/` | 22 rows each, exact success/test names/markers, expected type/assertion failures and recoveries |
| `interleaved/`, `interleaved-reverse/` | 141 rows each, three stacks, 42 measured edits per stack, all six permutations, exact execution markers, identical edit SHA per matched group |
| `profile-calibrated/` | 16 matched alternating rows, raw trace events/worker logs and their recorded SHA256 values, actual callback and transport receipts |
| `profile-failed/` | Original zero-accepted-row failure, raw first-row runtime output/trace, state and restoration identities |
| `build/{v22,v23}/` | All selected frozen source bytes and selected-source manifests (11 and 12 files), source boundary receipts, build logs, engine version/build identity and binary SHA receipts |
| `source-ready/` | Original helper, formatted payload, initial source-only doc, exact incident/removal receipt, unchanged historical hashes |
| `harness/`, `provenance/` | Original capture runners/build orchestration and harness sources |

The source snapshots are the exact **selected** native build inputs, not an
archive of the entire resident repository/toolchain. Toolchain/source identity
maps in runtime state are recorded before/after maps. Binaries and toolchain
artifacts are not copied into this bundle, so portable replay audits recorded
identities, not the original executable bytes. The collection verified copied
raw artifact bytes when those paths were available. Bundle seals are integrity
checks, not cryptographic authenticity guarantees.

## Results derived again from raw rows

Fixed-five engine median: 108.325347 ms old versus 107.247475 ms new, about
-1.00%, below the requested 10% floor. Reference median changed from 104.575675
to 129.957443 ms. The normalized -20.33% result is confounded and is not
accepted as a speedup. The original comparison output is retained verbatim.

| Independent interleaving | Old median ms | New median ms | Blaze median ms |
| --- | ---: | ---: | ---: |
| Original order | 106.2422585 | 93.0395020 | 99.9307425 |
| Reversed order | 106.8687520 | 100.7794350 | 105.9731245 |

The calibrated profile starts both daemons with diagnostics stats enabled,
prepares both, and alternates old/new order over eight matched edits. The
selected file has 373 callbacks per measured row. Actual old cumulative
callback intervals range 7,627,533 to 9,105,572 ns versus new 326,941 to
506,480 ns. No transport receipt has dropped spans. Both first rows additionally
report a distinct 231-callback file, with transport `retained=7 stale=94 dropped=0`.
Later rows report `retained=4 stale=0 dropped=0`. These extra-file callbacks and
stale spans are retained and checked, not silently excluded from the archive.
The selected-file ranges above identify their scope explicitly.

Callback time includes source lookup, text hashing, and the selective predicate.
It is inclusive in semantic diagnostic time. It is not hashing-only time,
additive critical-path savings, or an explanation for the whole semantic gap.
Profile wall durations are not substituted for the disabled-instrumentation
interleaved wall timings.

The original uncalibrated attempt failed with
`ValueError('missing positive diagnostics spans')`, zero accepted rows, and
restored source/toolchain/binary identities. The raw trace actually lacks the
required positive span coverage. The calibrated attempt is separate, not a
rewriting of that failure as a pass.

## Raw checks and tamper tests

The audit recomputes hashes and medians, parses each actual native test name and
result, checks runtime markers/test names/failure controls, checks paired edit
identities and permutation coverage, and compares raw profile traces and worker
receipts with recorded per-row identities. It validates selected source hashes
and verifies only main plus the new helper differ between snapshots. It never
uses the human-readable `justrust` report as a substitute for raw native output.

Nine tests include copied-path portability, missing runtime marker, missing raw
native passing line, dropped transport, missing trace span, changed interleaving
order, failure relabeled as pass, changed source bytes, and unsealed extra file.
Most semantic mutations update ordinary collection/inventory and per-row hashes
before replay, ensuring the checks reject semantic damage rather than merely
stale top-level hashes. Tests do not establish language correctness beyond the
retained original native test execution.
