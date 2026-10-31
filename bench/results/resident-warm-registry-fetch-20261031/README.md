# Warm registry fetch v85: rejected promotion

2026-10-31. Licensed Blaze-derived experiment. The shipping source is restored.
This archive is evidence, not a deployed implementation or a speedup claim.

## Result

| Gate | Actual result |
| --- | --- |
| Native registry / manifest / metadata / codegen / resident | 41 / 22 / 0 / 53 / 373 passed |
| Activation profile | 8 actual runtime rows |
| Strict OFF / ON / ON / OFF | 28 transitions passed |
| Pinned factor | 54 calls, 20 timed samples, 24 fresh nonces |
| OFF median seconds | 0.05607778349076398 |
| ON median seconds | 0.05501184299646411 |
| Descriptive difference | -1.900824939836343% |
| Decision | REJECT: below requested 10% floor, noise unknown |

All first touches are retained. Both candidate arms retain the same worker within
each segment. Whole-generation and watcher are fixed ON, so this is a combined
retouch factor, not separate manifest/lock attribution. Five pinned stops and four
post-measurement strict stops are recorded. Stores were retained. No significance,
general speedup or default promotion is claimed. The fixed batch marker proves
activation, not completion or latency. Native execution was coordinator-owned.

## Contract and provenance

V84 direct use of the mutating predicate was rejected. V85 independently proves
all markers read-only under the original package-cache lock, with exact first-use
`RA_JIT_WARM_REGISTRY_FETCH=1`, bounded inputs, and unchanged original fallback.
Unknown/error means original scheduler, never a new acceptance or early error.
Complete original/candidate source, original frozen filesystem oracle and patch
are retained in `frozen/`. All five new filesystem tests ran in the registry gate.
Shipping download.rs was restored bytewise to its accepted original, SHA-256
`9a9a1aa3979995c4429c8e4216c09a314ede844e61ba643ed77222f3aa1bf106`.
Tested candidate SHA-256 is
`ffcbba19b090e489d868bce73a9b5ead5885cb312ea778f8c632506d71e3c11d`.
Historical source-only RELEASE/STOPEDITS are preserved as historical records, not
claims that the subsequent coordinator execution did not occur. V83 is untouched.

Original strict semantic functions are AST-extracted from full archived original
oracles. Raw stdout is reparsed into original tuples, not weakened JSON lists.
Source63, toolchain4020, native-source3757, fixtures, native named outcomes, full
strict states, raw sample timings, worker PID/start ticks/argv/executable/socket
receipts and patch joins are checked offline. The private frozen adapter retains
its 18-mock PASS receipt, distinct from actual native tests. A restored full
harness receipt records 761 tests passing. Historical launcher failures
968422e0vc (wrong comparison path) and 403413eyir (argparse attribute) occurred
before native jobs. Initial and corrected launchers are archived, with the
attribute-only repair checked. Original private journal SHA/size remain recorded.

## Offline replay

Run from this directory, using only Python standard library:

```sh
python3 -B validate.py --manifest
python3 -B test_offline.py
```

No native jobs, network, cache locks or installations are needed. Tests include
coherently resealed mutations, actual raw/parsed joins, first-touch retention,
activation requirements and an isolated empty HOME/PATH replay. An audit hook
denies external reads, all writes, native execution, network and dynamic loads.
Denial controls exercise each boundary. Fixed observation SHA in validate.py binds
history even if an attacker coherently rewrites local JSON and inventories.
SHA256.json alone is a reproducibility inventory, not external authentication.

The initial offline run was 12 PASS / 1 failure: a negative test expected the
wrong rejection message for a coherently resealed no-op nonce. The validator
correctly rejected at its historical observation commitment. The incremental
test-only repair and initial transcript are retained in checks/. No oracle or
validator was weakened. Final replay and complete rerun receipts are there too.

## Privacy and licensing

export_private.py uses explicit field and line allowlists. Exported private
receipts retain original SHA-256 and byte sizes, selected diagnostics/test lines,
fixed startup controls, numeric traces and required process identities. No ambient
environment dump, private source tree, binary, cache/store contents, arbitrary
proc dump or unrestricted runtime log is published. Paths in derivative frozen
documents and drivers use role placeholders. Pinned-source contains complete
already-public git blobs, including their historical literal home-path strings.
The exact initial offline failure transcript also retains this public checkout
path solely in traceback frames. These are disclosed exceptions, not exported
private workload paths. Full raw private input journals remain retained privately.
Blaze attribution and MIT/Apache license texts are in lineage/. Public pinned
source and original oracle attribution are retained, not relicensed.
