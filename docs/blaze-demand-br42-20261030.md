# br42 demand-query-cache runtime evidence, 2026-10-30

The demand-query-cache hypothesis is **not a measured latency win**. Keep the experiment default-off. This descriptive seeded-warm filtered-test diagnostic supports no speedup, statistical significance, general performance claim, cold support, or default promotion.

| Stack | Median edit-to-filtered-test wall time, seconds |
| --- | ---: |
| Owned experiments off | 1.421465 |
| Owned all three earlier experiments on | 1.375786 |
| Owned demand-query cache only | 1.430772 |
| Blaze, pinned Cranelift backend | 0.106292 |
| Blaze, pinned LLVM backend | 0.105637 |

The frozen compiler driver SHA-256 is `ed21691a9db267ffb71cc3b8975a8cd89768397a7a7f82c9c839083485424efd`. All owned stacks use this same recorded driver identity. `owned-demand` adds only `-Zjr-demand-query-cache=yes` to the three earlier experiments explicitly set to `no`. Separate benchmark sessions and compiler revisions are not a controlled before/after comparison. Native correctness and root's subsequent dp43 mechanism profiling are separate evidence, not results established by this latency bundle.

## Protocol and retained evidence

Five edits per stack produced 25 timed rows. Preparation, type-error rejection, type recovery, deliberate test failure, and test recovery add 25 untimed gate rows. Every successful row runs exactly four `summary::tests::` tests with 144 filtered out. Every timed row emits exactly its own `JR_RUNTIME_EXECUTED_1` through `_5` marker once. The deliberate test failure affects only `shares_overlapping_time`. Type rejection requires E0308 and the injected marker. Stack order rotates between edits. Noise records explicitly mark contamination unknown and cannot establish a quiet machine.

`bench/results/blaze-demand-br42-20261030/` retains the completed private br42 root JSON, original edited file, stdout, stderr, all 30 owned raw-output journals, and the historical `launch-runtime42.sh`. No targets or caches are retained. Historical private paths are provenance, not dependencies for portable replay. The launcher is **not** the replay command.

```sh
python3 bench/results/blaze-demand-br42-20261030/validate.py
```

The standard-library-only validator runs no native commands and reads no private paths. It audits the complete artifact inventory and SHA-256 hashes, reconstructs every edited source hash, independently reparses runtime tests and exact markers, checks negative/recovery gates in order, checks 50 unique row keys and the rotated 25-edit schedule, compares recorded before/after source/tool/seed identities, pins the frozen driver and recorded backend/optimization flags, recomputes all five medians, and matches all 30 exported run IDs. Receipt replay is not fresh binary inspection or independent instruction-level backend attestation. The one-shot `seal.py` utility created the checked-in hash inventory and refuses to overwrite it.

All 30 owned invocations, including negative gates, were exported from three private recording homes, ten per home:

- `data/runs/xps14-runtime-br42-owned-off-20261030/2026-10.jsonl`
- `data/runs/xps14-runtime-br42-owned-on-20261030/2026-10.jsonl`, corresponding to `all3on`
- `data/runs/xps14-runtime-br42-owned-demand-20261030/2026-10.jsonl`

Byte-identical copies inside the evidence bundle make it portable. If corresponding repository exports exist, the validator checks their equality as well. The exporter redacted scratch runs as private. The unabridged raw-output journals remain the runtime evidence rather than slim exports or lossy console rendering. Raw stdout is retained byte-exact, including terminal blank lines.

## Limits

Both Blaze stacks started from the same recorded br30 seed-store content. **Seeded-warm only**: cold support remains blocked by Blaze writable statics. Five samples of four filtered tests do not establish statistical significance, full-suite/build/check performance, cold latency, or session-weighted performance. No defaults are promoted and no overall performance claim is made. No native/build/profile commands were executed for retention or validation. Pinned source, harnesses, root FINDINGS, earlier evidence bundles, and existing machine export directories were not edited by this publication.
