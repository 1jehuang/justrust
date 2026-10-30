# v30 state matrix: original exclusions and separate intrinsic derivative

Root executed `/home/jeremy/rsm02` with the unchanged strict frozen5ec494e60
matrix harness, complete pinned63-file `f97fcc2` source, same pinned nightly
2026-10-27 toolchain and controls. Reference is the frozen native ra-jit. Candidate
is v30 jr-engine SHA256
`c2606d25eb820ee515f5c5ae9d42f803916a4e3af974adb872c2fc4ba31c4e8b`.
The bounded cold-diagnostic fix/build proof is retained separately in
`../resident-cold-diagnostics-20261030/`, source fix commit91220ff43. The original
failed v28 matrix remains independently preserved in
`../resident-state-matrix-20261030/`. None of those originals is overwritten.

## Exact result, not a passing cross-engine matrix

All140 ordered native calls completed, with five replicates per stack,14 phases,
rotated stack order, reused per-session stores, source/manifest restored and all
tools/TC/raw identities unchanged. Each stack has65 cross-engine-admitted
observations and five **excluded** type-error observations. Original matrix
status remains `failed/incomplete`, exit1. Both type-error medians stay excluded.

* All five reference type-error calls correctly refused with status101, but only
  generic HasErrors. They fail current-source E0308/nonce freshness and remain
  rejected. Generic refusal is not diagnostic freshness.
* Each candidate type-error raw actually contains E0308 plus its unique current
  source nonce, status101, no parsed test execution and no runtime marker. The
  original unchanged cross-engine gate nevertheless excludes all five **solely**
  because the reference state oracle failed/unavailable. Their original
  `accepted:false` and original failure list remain unchanged.
* `intrinsic-typeerror.json` is a visibly **separate offline derivative**. It links
  each candidate raw receipt, nonce and exact mutated source hash, and invokes the
  original unchanged `matrix.oracle` on those same recorded bytes without adding
  the cross-engine reference-unavailable guard. All five intrinsic failure lists
  are empty. This does not admit any original cross-engine observation, rewrite
  metadata, create new native executions, promote medians, or establish an accepted
  complete matrix.
* Other130 observations pass exact per-state runtime markers/invocation counts,
  test names/statuses and recovery checks. Full-lib reference establishes148 exact
  names,147passed/0failed/1ignored/0filtered, and candidate matches. The sole
  ignored name is pinned `split_index::tests::attach_cost`, never a general ignore
  allowance.

## Descriptive per-state medians

Milliseconds, five admitted samples per listed cell. Explicit missing/error states
are not silently dropped. No weighted aggregate or significance inference.

| State | Reference ms | v30 engine ms |
|---|---:|---:|
| cold-prepare | 33754.508 | 33909.100 |
| repeated-noop-1 | 90.971 | 60.404 |
| repeated-noop-2 | 37.344 | 36.610 |
| summary-testbody-edit | 225.839 | 227.309 |
| summary-edit-recovery | 135.996 | 132.453 |
| ordinary-shim-edit | 123.072 | 123.840 |
| signature-base | 106.422 | 115.279 |
| signature-invalidation | 238.670 | 213.160 |
| manifest-feature-add | 6886.385 | 6397.977 |
| type-error | EXCLUDED (0/5 cross-engine) | EXCLUDED (0/5 cross-engine) |
| type-recovery | 5907.117 | 5703.163 |
| assert-failure | 1707.451 | 1620.055 |
| assert-recovery | 387.210 | 367.120 |
| all-lib | 870.280 | 926.606 |

Cold includes actual elapsed preparation with fresh private tool/dependency state,
not flushed OS page cache, HOME, toolchain or global disk caches. Timed states are
ordered and reuse stores. Workload markers, nested signature probes and manifest
mutations change inputs. Noise proxies explicitly do not establish quietness or
account for all resident-daemon CPU. Mixed medians, including slower cold/full-lib,
do not show general faster-than-Blaze behavior. This is not the comprehensive
session-weighted synthetic workload, a default-promotion gate, a general speedup,
a before/after suite comparison or statistical significance.

## Portable replay, exact originals

Copy this entire directory elsewhere, with no Git, sibling bundle, engine,
toolchain or original private paths required:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 bench/results/resident-state-matrix-v30-20261030/validate.py
PYTHONDONTWRITEBYTECODE=1 python3 bench/results/resident-state-matrix-v30-20261030/test_validate.py
```

PASS means faithful offline replay of the **failed** original cross-engine matrix
and the explicit intrinsic-only derivative, not full matrix acceptance. Source
recipes and parser/oracles are imported from retained independent-hash-pinned
helpers. Replay checks140 exact calls,10 version calls, all state/source/transition
hashes, private environments, tool/TC identity, raw size/hash/parser, runtime
nonce/invocation counts, actual reference contracts, every failure/admission,
original medians, manifest backup and durable restoration. Re-signed mutation and
empty-HOME relocation controls reject credential injection, stale nonces, changed
counts/summaries, false restoration, source/helper drift, or promotion of either
reference negatives or candidate cross-engine exclusions. They also reject edits
to the separate derivative's nonce or cross-engine-admission flags.

`--derive` recomputes only `intrinsic-typeerror.json` offline, then reseals the
portable inventory. It never changes retained original run JSON/raw. `--seal` is
only a maintenance checksum operation, not a signature or authenticity claim.

`runs/rsm02` retains all raw console streams, version streams, original source and
manifest backups and sanitized metadata/records/summary. `input/source63` retains
all63 exact source files/Git blob inventory, and `provenance/` the frozen helpers.
No binaries, cache trees, targets, private stores or ambient credentials copied.
Environment maps are explicitly allowlisted derivatives, including nested version
receipts. Original private byte commitments and derivative hashes are retained in
`transformation.json` and `evidence.json`; export rechecks originals unchanged.
Diagnostic text and historical paths remain visible, so this is not path-anonymized.
Checksums establish reproducibility, not independent authenticity or privacy beyond
the inspected allowlist and omitted-sensitive-value checks.
