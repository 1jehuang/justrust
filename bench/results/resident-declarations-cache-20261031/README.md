# Resident declarations cache, 2026-10-31: REJECT, no promotion

This is historical evidence, not an enabled optimization or a clean-room engine.
The coordinator restored the live main.rs/frozen.rs originals after v41.
No speedup is claimed. Native correctness passed, but useful robust gains did
not exceed the floor and assertion recovery regressed materially.

| Transition | First pair on relative to off | Reverse-order pair |
|---|---:|---:|
| Cold | +2.26% | +2.18% |
| Manifest feature add | -0.96% | -3.14% |
| Type recovery | -2.12% | +0.03% |
| Assertion failure | +1.72% | +4.95% |
| Assertion recovery | +22.29% | +10.03% |

Positive is slower. Noise is unknown/uncontrolled. These focused native state
transitions are not the comprehensive session-weighted synthetic benchmark.
Exact elapsed values and calculated percentages are in validation.json, derived
from raw state metadata. First pair ran on then off, reverse pair off then on.

## Retained records

* frozen/v39: exact accepted originals, first full candidate and patch, unchanged
  stable.rs original/candidate and original source-ready documentation/hashes.
* frozen/v40: coordinator's actual compiler repair, including tracked copy return
  and two explicit DefWithBodyId conversions. v39 had four real compiler errors.
* frozen/v41: full candidate, v40 original, incremental patch and original docs.
  hir-def-frozen.rs is the unchanged native portable codec used by these builds.
* gates: actual check/build/unit logs, exits, binary identity/version receipts,
  lock receipts, all 3757 source identities and after-build identity receipts.
* journals: complete meta.json, output.jsonl and summary.json for nine actual
  justrust runs. v40 passed 48 codegen and 368 resident tests. v41 passed 48 and
  369. These include original-vs-cache fixtures with fresh database remapping,
  nested blocks/macros/statics, duplicate/order/body-queue preservation,
  corrupted/truncated records, source/cfg/dependency changes and macro-read
  environment diagnostics. v41 additionally tests observed-read env witnesses.
* slim: path-free derived metadata exports, joined back to the raw journals.
* runtime: all stdout/stderr/worker snapshots, state.json and durable original
  input bytes for six seven-transition sessions (42 rows). v40 contributes 14
  rows and v41 contributes 28. No engine binary or live private cache is retained.
* provenance: exact native build scripts, runtime runner, pure parsing/oracle
  helpers, and collection receipt. Scripts are historical inputs, not audit jobs.
* input/source63: exact pinned workload inputs, including original licenses.
* licenses: retained MIT/Apache attribution for licensed Blaze-derived sources.
  The engine's exact Blaze origin and build version remain in native receipts.

Each v41 on cold worker snapshot has 21 actual native-hit markers. All subsequent
snapshots repeat that cumulative history, not 21 new hits per state. The v40-on
cold/noop snapshots also have 21, then zero after manifest reload. Earlier claims
of zero throughout v40 were contradicted by the retained raw snapshots. The
validator makes this correction explicit and never treats absence as a hit.

## Offline audit only

From any relocated copy with Python3 and its standard library:

	python3 -B validate.py
	python3 -B test_validate.py

Neither command starts a compiler,
engine, benchmark, Git process or network request. validate.py checks transport
inventory, independent frozen-source/helper commitments, actual patch replay,
native exits/test names, engine joins, source and toolchain restoration, all
42 strict state oracles, invocation-specific runtime challenges, error/recovery
semantics, input recipes and cumulative native counters. Existing absolute
paths are provenance strings, never opened by the replay.

test_validate.py copies the bundle and forbids original-checkout file reads,
subprocess launches and sockets while clearing HOME/PATH. It validates the
relocated copy and rejects 17 semantic corruptions even after regenerating the
outer SHA256 inventory. Raw-output controls also regenerate nested byte/hash
receipts, so rejection is not merely a transport-checksum failure. Coverage:
startup flag, missing transitions, source/tool/toolchain identity, input recipe,
actual parsed names, unsupported environment keys, noise claims, invalid elapsed
time, process status, restoration, runtime invocation marker, E0308 diagnostic,
assertion marker, native fixture coverage and positive native-hit marker.

SHA256SUMS is a resignable transport inventory, not an authenticity signature.
The independently pinned commitments protect historical sources/helpers against
merely re-signing this inventory. validation.json is derived and deliberately
excluded from the inventory. All nested historical SHA receipts are included.
Audit output is retained in audits/. No ambient environment values were captured
by collection: journal/runtime environments already present in the frozen runs
are checked against explicit field allowlists. Full process environments,
credentials, telemetry homes, cache databases and binaries were not copied.

The doc docs/resident-declarations-cache.md describes historical certificate,
portable-ID and fail-open design. Passing these evidence audits does not replace
native tests or establish an optimization benefit.
