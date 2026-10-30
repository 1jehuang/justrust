# Native backend frontier br44, 2026-10-30

These are **descriptive focused-loop medians only**, not a statistical speedup, significance, general workload, or default-promotion claim. The owned-versus-Blaze median gap remains greater than 10x even for the lowest owned median. No defaults change.

| Stack | Median edit-to-filtered-test wall time, seconds |
| --- | ---: |
| Owned experiments off | 1.288858 |
| Owned all three earlier experiments on | 1.292491 |
| Owned native Cranelift | 1.284490 |
| Owned threads=4 | 1.198348 |
| Owned native Cranelift plus threads=4 | 1.186774 |
| Blaze, pinned Cranelift backend | 0.100719 |
| Blaze, pinned LLVM backend | 0.111574 |

All five owned stacks use the same frozen compiler driver, SHA-256 `ed21691a9db267ffb71cc3b8975a8cd89768397a7a7f82c9c839083485424efd`. The native Cranelift shared library is independently recorded in tool identities with SHA-256 `4d5659e20a9d943765e0a7eceb88301aaf9b8dcfbf6adfe6357938299023d2c7`, unchanged before and after. Its path ends in `codegen-backends/librustc_codegen_cranelift-1.101.0-nightly.so`.

The three new stacks start with the earlier three experimental flags explicitly `no`, adding exactly `-Zcodegen-backend=cranelift`, `-Zthreads=4`, or both in that order. `all3on` sets the earlier three flags to `yes`. Blaze's recorded backend is separately pinned to `cranelift` or `llvm` with `BLAZE_OPT=0`. Receipt replay is not independent instruction-level backend attestation. Do not treat separate br42/br44 sessions as controlled before/after measurements.

## Protocol and evidence

Seven accepted stacks, five timed edits each, produce **35 edit rows and 70 total rows**. Each stack has five untimed gates: prepare, type-error rejection, type recovery, intentional test failure, and test recovery. Success requires exactly the same four `summary::tests::` tests with 144 filtered out. Every timed row emits precisely its own `JR_RUNTIME_EXECUTED_1` through `_5` marker once. The intentional failure affects only `shares_overlapping_time`. Type rejection requires E0308 and the injected marker. The seven-stack order rotates across five edits. Noise records classify contamination as unknown and cannot establish a quiet machine.

The dedicated `bench/results/native-backend-frontier-br44-20261030/` directory retains completed private br44 root JSON, the original edited source, stdout, stderr, all **50 owned raw-output journals**, and historical `launch-runtime44.sh`. No targets or caches are retained. Historical absolute paths are provenance only, not portable replay dependencies. The launcher is **not** the replay command.

```sh
python3 bench/results/native-backend-frontier-br44-20261030/validate.py
```

The standard-library-only validator invokes no native commands and reads no private files. It checks the complete artifact inventory and raw SHA-256 manifest, reconstructs all injected source hashes, independently reparses runtime tests and exact markers, validates negative/recovery gates and their order, verifies 70 unique row keys and the rotated 35-edit schedule, compares recorded before/after source/tool/seed identities, pins the compiler driver and native Cranelift shared library, checks exact flags/backend environment, recomputes seven medians, and matches all 50 owned run IDs against exports. The one-shot `seal.py` utility generated the checked-in inventory and refuses to overwrite it. Identity checks replay retained receipts rather than inspecting current binaries.

## Dedicated exports

All 50 owned calls, including negative gates, were exported from five private recording homes, ten per home, to `data/runs/`:

- `xps14-runtime-br44-owned-off-20261030/2026-10.jsonl`
- `xps14-runtime-br44-owned-on-20261030/2026-10.jsonl`, corresponding to `all3on`
- `xps14-runtime-br44-owned-clif-20261030/2026-10.jsonl`
- `xps14-runtime-br44-owned-threads4-20261030/2026-10.jsonl`
- `xps14-runtime-br44-owned-clif-threads4-20261030/2026-10.jsonl`

Byte-identical copies within the evidence bundle permit standalone replay. If corresponding repository exports are present, their bytes are also checked. The exporter redacted scratch runs as private. Raw journals, not slim exports or lossy console rendering, are the authoritative runtime output. Raw terminal blank lines are preserved.

## Limits

**Seeded-warm only**. Both Blaze stacks use the same recorded br30 seed-store content. Cold support remains blocked by Blaze writable statics. Five samples of four filtered tests establish neither significance nor full-suite, check/build, cold-start, or session-weighted performance. These backend/thread observations do not certify general Rust compatibility or justify default promotion. Native correctness remains separate evidence. No native/build/profile commands were run for this retention task. Pinned source, harnesses, earlier evidence, root FINDINGS, and existing machine export directories were not edited.
