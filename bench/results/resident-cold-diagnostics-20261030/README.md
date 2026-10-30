# Cold diagnostic ordering: separate v30 correctness evidence

This bundle is separate from the original failed state matrix retained at
`../resident-state-matrix-20261030/`. It never relabels that matrix's ten rejected
type-error calls. Root owns the native jobs and the native source fix in commit
`91220ff43`. The evidence worker made read-only copies and offline replay code,
not native runs. The engine is licensed imported Blaze-derived code, not the
clean-room compiler and not the installed shipping client.

## Exact bounded observation

The unchanged runner compares **v28 jr-engine** (historical `reference` label)
against **v30 jr-engine** (`engine` label), not reference ra-jit against v30. Each
gets a separate fresh private namespace, the same complete pinned63-file
`f97fcc2` source and nightly2026-10-27 toolchain. Ordered calls reuse their own
stores: initial-cold type-error, recovery, warm type-error, recovery. Every error
must refuse with status101, exact current-source E0308 and unique nonce, and no
runtime. Every recovery must execute the exact four baseline tests and its unique
current runtime marker. No generic HasErrors, stale nonce or exit0-only recovery
is accepted.

The original longer-output-root experiment `/home/jeremy/rstp03` is retained
unchanged, including all failures. Both v30 errors supplied precise diagnostics,
but both engines' recoveries hit Unix socket SUN_LEN limits (112 versus109bytes).
No overall candidate correctness success is inferred from that setup failure.

The **same exact script, source recipes, tools, controls and oracle** ran again
with the shorter output root `/home/jeremy/p`:

| Stack | Initial cold error | First recovery | Warm error | Second recovery |
|---|---|---|---|---|
| Frozen v28 engine | REJECTED: generic HasErrors | accepted | accepted: E0308 + nonce | accepted |
| Frozen v30 engine | accepted: E0308 + nonce | accepted | accepted: E0308 + nonce | accepted |

All invalid-source invocations refused. The successful bounded differential is
**seven admitted observations, one mandatory baseline cold rejection**, not eight
admitted cases. Both runners exit1 because the historical negative baseline is
intentionally not admitted. Source/manifest restoration and tool/toolchain/raw
identity gates pass. This supports the bounded initial-cold diagnostic correctness
fix, not performance, general superiority, statistical significance, default
promotion, or acceptance of the full state matrix.

## Source and build proof

`proof/` contains original and frozen repaired main.rs, original licenses, and an
exact unified diff. Portable replay applies that diff without fuzz or a process,
checks both independent source hashes, and verifies that **only** the fallback
block moved (plus its explanatory comments). The existing whole-file detailed
source check now precedes generic cold refusal. Refusal remains before linking
and execution. No source-check semantics, cache policy or invalid-source admission
was changed in this patch.

`journals/` retains the root orchestration script, check/build exits and full logs,
engine version/hash, complete crates/vendor before-source inventory, exact after
identity check logs, Cargo.lock identity checks, full native359-test exit/log,
and all eight explicit justrust journal files for each of the three gates.
The full journal test output is replayed as359 unique named passing tests with
zero failed, ignored or filtered tests, not just a summary slogan. Artifact SHA256
links the checked source build to the actual v30 probe metadata. No binary,
build target, dependency/private cache tree or justrust-home installation copied.

`runs/` retains all16 native console pairs, source/manifest backups, worker initial
snapshots and percall before/after/deltas. Both experiments' original private
metadata/records/summary commitments are pinned independently. Sanitized JSON and
JSONL are separate derivatives with an explicit environment allowlist. Original
private bytes remain unchanged. Full diagnostic text, test output, historical
paths and noncredential provenance remain visible. This is not path-anonymized.
Checksums establish reproducibility, not a cryptographic authenticity attestation
or an absolute privacy guarantee beyond the checked maps.

## Portable offline replay

After copying this whole directory to any location, with no original private paths,
Git checkout, sibling bundle, engine or toolchain needed:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 bench/results/resident-cold-diagnostics-20261030/validate.py
PYTHONDONTWRITEBYTECODE=1 python3 bench/results/resident-cold-diagnostics-20261030/test_validate.py
```

The trusted retained parser/helper sources are independently hash-pinned before
import. The validator executes no Git, compiler, engine or build. Its PASS means
faithful semantic replay of the exact bounded differential, including the earlier
path-limit failures and mandatory baseline cold rejection. The tests relocate into
an empty HOME and reject coherently re-signed mutations to raw current nonces,
acceptance, source inputs, exact four-test recovery counts, tool identities,
restoration, credentials, native359 status/names, source patch and runner identity.
`--seal` is a maintenance checksum operation, not admission or a trusted signature.

Cold here means fresh private engine/dependency stores, not flushed OS page cache,
HOME, toolchain/global disk cache. Ordered states and runtime probes mutate the
source/workload. No timing medians or speedup claim are drawn from these eight-call
diagnostics. Any later five-sample matrix is a new experiment with its own original
receipts, not a replacement for these failures or the original failed matrix.
