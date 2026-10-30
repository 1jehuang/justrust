# br40 TT declarative-cache runtime evidence, 2026-10-30

Keep the cache **default-off**. This seeded-warm filtered-test diagnostic establishes no speedup, statistical win, cold support, general workload result, or default promotion. The approximately 12 ms difference between off and TT medians is not a statistical win. The observed owned-versus-Blaze median gap remains greater than 10x in this focused loop, not a general performance claim.

| Stack | Median edit-to-filtered-test wall time, seconds |
| --- | ---: |
| Owned experiments off | 1.229047 |
| Owned all three earlier experiments on | 1.171899 |
| Owned TT declarative cache only | 1.217220 |
| Blaze, pinned Cranelift backend | 0.101007 |
| Blaze, pinned LLVM backend | 0.093102 |

The new frozen compiler driver SHA-256 is `ac116b9398e1f5c677e73952b8719be9d9e47190fe61fbaeba9b7cf270739595`, distinct from br37. Do not infer an improvement by comparing separate br37 and br40 sessions. Native correctness evidence remains separate from these runtime observations.

## Protocol and retained evidence

Five edits per stack produced 25 timed rows. Preparation, type-error rejection, type recovery, deliberate test failure, and test recovery add 25 untimed gate rows. Each successful row runs exactly four `summary::tests::` tests with 144 filtered out. Every timed row emits exactly its own `JR_RUNTIME_EXECUTED_1` through `_5` marker once. The deliberate test failure affects only `shares_overlapping_time`, and type rejection requires E0308 and the injected marker. Stack order rotates between edits. Noise records mark contamination unknown and do not establish a quiet machine.

`bench/results/blaze-declarative-br40-20261030/` retains completed private br40 root JSON, original edited source, stdout, stderr, all 30 owned raw-output journals, and the historical `launch-runtime40.sh`. No targets or caches are included. Historical absolute paths record provenance only. The launcher is **not** the portable replay command.

```sh
python3 bench/results/blaze-declarative-br40-20261030/validate.py
```

The standard-library-only validator runs no native commands and reads no private paths. It audits the complete artifact inventory and raw hashes, reconstructs every edited source hash, independently reparses exact runtime tests and markers, checks negative/recovery gates in order, checks 50 distinct row keys and the rotated 25-edit schedule, compares recorded before/after source/tool/seed identities, pins the TT driver and recorded backend/optimization flags, recomputes all medians, and matches 30 exported run IDs. These are receipt replays, not fresh binary inspection or independent instruction-level backend attestation. The one-shot `seal.py` utility created the checked-in hash inventory and refuses to overwrite it.

All 30 owned calls, including negative gates, were exported from three private recording homes, ten per home:

- `data/runs/xps14-runtime-br40-owned-off-20261030/2026-10.jsonl`
- `data/runs/xps14-runtime-br40-owned-on-20261030/2026-10.jsonl`, corresponding to `all3on`
- `data/runs/xps14-runtime-br40-owned-declarative-20261030/2026-10.jsonl`

Byte-identical export copies inside the bundle permit standalone replay. If corresponding repository exports are present, the validator also checks equality against them. The exporter redacted scratch runs as private. Slim exports and lossy console output do not replace the unabridged raw journals.

## Limits

Both Blaze stacks were seeded from the same recorded br30 store content. **Seeded-warm only**: cold support remains blocked by Blaze writable statics. Five samples of four filtered tests cannot establish significance, full-suite/check/build performance, cold latency, or session-weighted performance. No default is promoted and no overall performance claim is made. Native correctness and root's subsequent profile work are separate and are not rerun or modified by this publication. No native/build/profile commands were executed, and pinned source, existing harnesses, root FINDINGS, and existing machine export directories were left untouched.
