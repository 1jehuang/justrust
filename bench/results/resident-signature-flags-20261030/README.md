# Resident signature-flag experiment, evidence only

**Decision: NOT A PERFORMANCE WIN. No source promotion or source-restoration claim.**

This archive records a bounded native optimization in the licensed Blaze-derived resident engine, not the clean-room compiler. It stores `rustc_comptime` in bit 15 of existing `FunctionSignature` flags using the exact original `AttrFlags::has_word` predicate, and makes codegen read that signature. It adds no tracked query nodes.

## Recorded outcome

| Gate | Original result retained |
| --- | --- |
| v13 native production check/build | PASS |
| v13 `hir-def` fixture gate | FAIL, exit 101: excluded workspace dependency cannot be tested with its dev-dependencies. The successful build's exit 0 is distinct from the later orchestration exit 101. |
| v14 integrated fixture check | FAIL, E0502, exit 101: database-borrowed baseline persisted across mutable file revisions. Runtime blocked with exit 2. |
| v15 owned-baseline repair, native check/build | PASS |
| v15 signature fixture | 1 actual test passed, asserting 12 distinct functions × 8 revisions = 96 direct-oracle and codegen comparisons |
| v15 declaration graph / listed roots / telemetry | 1 / 1 / 3 actual native tests passed |
| v15 complete native suite | **347 passed, 0 failed/ignored/filtered**, 16.781 s recorder wall time (displayed 16.8 s) |
| Runtime controls, RSB04 and RSA10 | Type error E0308, recovery, real assertion failure, recovery, exact four test names, and five distinct executing edit markers passed for both stacks |
| Scoped engine medians | **0.116184 s → 0.105814 s**, raw **−8.93%**, below the requested 10% floor |
| Reference-normalized ratio | **0.904701 → 0.924668**, **+2.21% worse** |

The historical per-command environment allowlist omits remote/proc-macro variables. Their explicit off/zero policy is retained and checked in session metadata, not claimed as an independently captured per-command environment attestation. Recorded per-command backend/profile fields are checked directly.

The timing pair is descriptive, with unknown contamination. It is neither a significant speedup nor the comprehensive session-weighted synthetic workload or default-promotion gate. All samples and failures remain present. No general speed, memory, cold-lowering, clean-room, or shipping claim is made.

## Portable offline replay

From any working directory, with Python 3 and no third-party packages:

```sh
python3 /path/to/resident-signature-flags-20261030/validate.py
PYTHONDONTWRITEBYTECODE=1 python3 /path/to/resident-signature-flags-20261030/test_validate.py
```

The validator resolves all inputs relative to this bundle, not to absolute historical paths inside receipts. It never invokes git, cargo, justrust, a native engine, or a network service. The positive relocation test copies the archive outside the repository and runs the validator with an empty PATH. Native tests and workloads were executed only by the coordinator. Offline replay and mutations are not new native test executions.

`audit.json` is the generated replay report, including all 347 actual raw test names. `offline-tests.log` records **47 passing offline checks**: two positive checks and 45 negative/mutation checks. Mutation checks deliberately bypass the outer inventory and, where needed, reseal redundant suite JSON receipts. This exercises independent semantic rejection rather than only detecting checksum changes. No native tool is needed to reproduce them.

### What is independently verified

- Exact artifact inventory and SHA256s for **266 captured evidence artifacts**, excluding audit implementation and generated reports from the inventory.
- Original source-ready SHA256 manifests for v13, v14 and v15, genuine root before-build frozen source snapshots, and all three original source patches. Patches replay in memory against archived genuine baseline revision `8905ce5931b352545ec4a18a3bbb8e6f13bd1b63`, producing the exact pinned candidate hashes. Main.rs changes in v14/v15 are test-only. No earlier full source is reconstructed.
- Eleven exact native run IDs, full recorded command/filter arguments, local route, exit status, timestamps, test counts and filtered counts, bound to the eleven supplied slim exports. The original raw recorder output, metadata, summaries and available unit receipts are retained.
- Real positive libtest admission and complete raw passing names. The full 347-name set has SHA256 `02e6c1fa64cff19fac2bdc2548001b598f011cee2f8a344b58c58f57efa41b0d`. Summary booleans cannot replace the raw oracle or hide zero selected tests.
- Failed v13 workspace and v14 borrow-check gates, the blocked v14 runtime gate, and intact source-after receipts for successful frozen builds/units/full native run.
- Both runtime sessions' complete top-level raw artifact receipts, exact four native workload test names and statuses, source edit hashes derived independently from the original workload bytes, positive type/assertion failure controls and recoveries, and unique ordered edit execution markers.
- Exact source-built reference and candidate executable hashes and version strings. Runtime candidate identity is bound to the original v15 build receipt. Source63 and full toolchain manifests are pinned and matched, as are host/CPU/execution identity, Cranelift profiles, remote-off/proc-macro compatibility controls, private unseeded stores and five samples per stack.
- Medians recomputed from original record timings, reference normalization and the exact archived comparison output. The decision remains `NOT_A_PERFORMANCE_WIN`, with `promotion: false`.

## Layout and provenance limits

- `source-ready/v13`, `v14`, `v15`: original private worker receipt contents, exact changed-source bytes, historical candidate documentation, `SHA256SUMS`, and `source.patch`. No original DM files were created, so none are invented here. The ready manifests are the original byte receipts.
- `native/candidate-build-v13..v15`: original check/build logs, statuses, source manifests, genuine root frozen source snapshots, and raw recorder runs. `native/candidate-unit-*` retains the original failed/passing test receipts. V14 has no successful unit run to invent.
- `native/baseline-v4`: available original baseline identity and source hash receipts, not an invented full v4 source reconstruction.
- `exports/`: supplied path-redacted slim exports, grouped by version/build/unit, bound by exact IDs to original raw runs. Original raw receipts retain their historical paths and names.
- `sessions/rsb04`, `sessions/rsa10`: all original top-level source-built runtime outputs, source backup, metadata, raw records, summaries and suite receipts. Private caches and build/engine binaries are excluded.
- `provenance/`: original build/unit/runtime orchestration scripts, status logs, admission script and comparison output. These are historical evidence, not scripts the portable validator executes.
- `baseline-head/`: actual git blobs from the captured baseline revision, used only for offline patch replay.
- `harness-captured/`: relevant harness files copied at evidence-retention time, not asserted to be original historical snapshots. The portable validator independently implements the checks it needs instead of executing that harness.
- `provenance.json`, `manifest.json`: explicit non-win decision, limits and exact immutable artifact inventory.
- `source-ready/LICENSE-*`: original resident Apache/MIT attribution. Vendored rust-analyzer sources retain their existing attribution.

The current working sources remain uncommitted experimental inputs under root and another worker's ownership for separate follow-on experiments. This evidence-only archive does not restore or commit those sources, does not record a binary promotion, and does not claim that all Rust semantics or diagnostics are proven by the recorded gates. Potential additional cold signature-lowering cost remains a reason not to infer a benefit from bitset reuse alone. See `docs/resident-signature-flags.md` for the implementation and semantic scope.
