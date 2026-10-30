# Native Cargo residency br45, 2026-10-30

**No residency win. No general performance or default-promotion claim.** These are descriptive medians from one seeded-warm filtered-test session, not statistical significance or a comprehensive before/after benchmark. No defaults change.

| Stack | Median edit-to-filtered-test wall time, seconds |
| --- | ---: |
| Owned, official Cargo, experiments off | 1.206150 |
| Owned, official Cargo, earlier three experiments on | 1.263922 |
| Owned native Cargo, daemon explicitly off | 1.177500 |
| Owned native Cargo, daemon on | 1.268064 |
| Owned native Cargo, daemon on plus `-Zthreads=4` | 1.296977 |
| Blaze, pinned Cranelift backend | 0.097911 |
| Blaze, pinned LLVM backend | 0.104013 |

The daemon medians are higher than the native non-daemon control in this session, despite all ten timed daemon edits retaining the exact `jr-cargo daemon: served by a parked server` string in their authoritative raw journals. The native control median is below the official-Cargo control median here, but five samples are not a demonstrated general speedup. The large owned-versus-Blaze focused-loop gap remains.

## Configuration and provenance

The optional frozen native Cargo binary has SHA-256 `62eecc8e8a6a54e3ea992c7fdf86eb58f416f0e81ebcd1ea837f9427a64eab93`, unchanged before and after. Its saved, untimed `-vV` receipt reports `cargo 1.101.0-nightly (f3865b2a4 2026-09-29)` with `jr-cargo d57511c1f21d60aff7be78590c503d165d375ed5`. Version-query hashes match the full-session identity receipts. The same owned compiler driver is used throughout: SHA-256 `ed21691a9db267ffb71cc3b8975a8cd89768397a7a7f82c9c839083485424efd`.

Each stack has its own HOME, target and XDG runtime/cache/config/data roots. The three native-Cargo stacks set `CARGO` and `JUSTRUST_REAL_CARGO` to the frozen native binary, also recorded as `real_cargo` in all thirty private recorder metadata files. The baseline stacks retain the official Cargo identity. The harness creates the native PATH cargo symlink, but PATH and symlink filesystem snapshots were not recorded in br45. Retained environment and recorder receipts are not independent instruction-level execution attestation.

The native control explicitly sets `JUSTRUST_CARGO_DAEMON=0`. Both daemon stacks set it to `1`. All three set `JUSTRUST_CARGO_DAEMON_TRACE=1`, keep the earlier three compiler reuse experiments `no`, and disable proc-macro caching, buildd, dependency/local/peer/remote/prebuilt caches and toolchain installation. Only `owned-jrdaemon4` adds `-Zthreads=4`. The separate `all3on` baseline sets the earlier three experimental flags to `yes`. Fixed dev/test profiles use opt-level 0, debug 0 and incremental true. Blaze backends are pinned separately to `cranelift` and `llvm`, both with `BLAZE_OPT=0`.

## Gates and retained evidence

The run exited zero. Seven stacks each passed prepare, intentional E0308 rejection, type recovery, intentional test failure and test recovery before five timed edits. This yields **70 rows, 35 timed edits, 50 owned recorded calls**. All timed rows were accepted, with no exclusions. Every success has the exact four expected `summary::tests::` tests, 144 filtered out, and no unexpected failures. Each timed edit prints exactly its own `JR_RUNTIME_EXECUTED_1` through `_5` marker once. The deliberate test failure affects only `shares_overlapping_time`. Stack order rotates across the five edits. Original source, reconstructed injected source hashes, before/after source and tool maps, and before/after Blaze seed manifests all agree. Noise is recorded as unknown, not a quiet-machine guarantee.

`bench/results/native-cargo-br45-20261030/` contains the completed root JSON, original edited source, all flat stdout/stderr and 50 raw journals, compiler/native-Cargo version receipts, historical launcher and launch/exit logs, all fifty private recorder state directories, and the two daemon runtime `.out`/`.err` pairs. Targets, caches, binaries and socket objects are intentionally not retained. Absolute paths inside receipts are historical provenance, not portable file dependencies.

Dedicated exports were made only from these five private br45 recording homes, ten calls each, into bundle-local `exports/`:

- `xps14-runtime-br45-owned-off-20261030/2026-10.jsonl`
- `xps14-runtime-br45-owned-on-20261030/2026-10.jsonl`, corresponding to `all3on`
- `xps14-runtime-br45-owned-jrcargo-20261030/2026-10.jsonl`
- `xps14-runtime-br45-owned-jrdaemon-20261030/2026-10.jsonl`
- `xps14-runtime-br45-owned-jrdaemon4-20261030/2026-10.jsonl`

No global export was used, and no existing `data/runs` directory was changed. Raw journals, not slim exports or condensed console rendering, are the runtime authority. Both raw-journal copies are byte-matched against their per-row SHA-256 and recorder state.

## Private cleanup receipt limitations

The final metadata reports two SIGTERM signal attempts, PIDs `2831071` and `2834548`. **These are not independently proven process exits or historical UID/executable/socket identity checks.** No historical `/proc` snapshot exists. The harness source and its strict mock-tested pidfd/private-root safety gate are separate evidence, not retroactive process attestation. Per-call `cargo-daemon.pid` files describe recorder-associated processes and must not be conflated with those reported parent cleanup PIDs.

The retained daemon stderr says `parked at` its private socket. The daemon4 stderr instead says `bind ...594a1184dc88989e.sock: server alive`. This is preserved verbatim, not recast as a parked-startup receipt. Eligibility rests on the exact served marker in **every timed raw journal**, not these startup logs. Cold preparation need not contain a served marker.

## Portable offline replay

```sh
python3 bench/results/native-cargo-br45-20261030/validate.py
python3 bench/results/native-cargo-br45-20261030/test_validate.py
```

Replay uses only the standard library and bundle files, from any checkout or copied directory. It never invokes a compiler, Cargo, justrust, Blaze or a daemon. The validator checks inventory and SHA-256 for every retained file including its own source, independently reparses exact test results and unique runtime markers, reconstructs source edits, validates negative/recovery gates, exact flags and private isolation roots, native Cargo/version/seed identities, ten raw served proofs, all 70 row keys and rotated edit order, medians, recorder metadata/status and all 50 export IDs. It replays cleanup **signal receipts only**. The mutation tests include semantic forgeries with updated raw hashes and matching recorder copies, proving that missing served markers, stale/duplicate runtime markers and broken rejection/recovery evidence are rejected independently of inventory checks. They also cover flags, identities, seed, timeouts, isolated homes, cleanup receipt changes, exports, state and missing/extra/tampered artifacts.

`seal.py` creates the one-shot inventory and refuses to overwrite it. `retain.py` and `launch-runtime45.sh` are historical retention/launch utilities, not replay commands. No native workloads were run while retaining this evidence. Only this new results directory and this new document were changed.

## Scope limits

This is a seeded-warm, four-filtered-test diagnostic. No full suite, cold support, checks/builds, statistical significance or session-weighted performance conclusion follows. The synthetic comprehensive gate was not run for this retention task, and this result is not a substitute for it. Supported Rust behavior and broader native Cargo correctness remain separate contracts. br45 should not be compared with another session as a controlled before/after result.
