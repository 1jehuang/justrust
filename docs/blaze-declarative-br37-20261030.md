# br37 declarative-cache runtime evidence, 2026-10-30

The new bounded literal declarative-macro cache **did not improve focused latency**. Keep it default-off. This is a descriptive, seeded-warm, filtered-test diagnostic, not a speedup, significance, general workload, cold-store, or correctness-certification claim.

| Stack | Median edit-to-filtered-test wall time, seconds |
| --- | ---: |
| Owned experiments off | 1.472484 |
| Owned all three earlier experiments on | 1.527366 |
| Owned literal declarative cache only | 1.580672 |
| Blaze, pinned Cranelift backend | 0.113914 |
| Blaze, pinned LLVM backend | 0.130430 |

Five edits per stack produced 25 timed rows. Preparation, type-error rejection, type recovery, deliberate test failure, and test recovery add 25 untimed gate rows. Every successful row runs exactly four `summary::tests::` tests with 144 filtered out. Every timed row emits exactly its own `JR_RUNTIME_EXECUTED_1` through `_5` marker once. The test-failure gate fails only `shares_overlapping_time`, while type-error rejection requires E0308 and the injected marker. Stack order rotates between edits. Noise records explicitly say contamination is unknown and cannot establish a quiet machine.

## Retained evidence and replay

The dedicated directory `bench/results/blaze-declarative-br37-20261030/` retains the completed private br37 root JSON, original edited file, stdout, stderr, and all 30 owned raw-output journals. No targets, compiler artifacts, or caches are retained. Paths in these diagnostic artifacts are historical provenance, not dependencies for replay. `launch-runtime37.sh` is retained as the historical launcher only, **not** a replay command.

Run the standard-library-only validator from any working directory:

```sh
python3 bench/results/blaze-declarative-br37-20261030/validate.py
```

It executes no native commands and reads no private paths. It checks the complete artifact inventory and SHA-256 manifest, independently reconstructs edited source hashes from `source.original`, reparses exact runtime tests and markers from raw output, checks both negative and recovery gates, verifies 50 unique row identities and the rotated 25-edit schedule, pins recorded compiler/Cargo/client/Blaze identities and flags, compares before/after source/tool/seed identities, verifies both Blaze seed manifests, recomputes the five medians, and matches all 30 owned run IDs to exports. Identity checks replay recorded receipts, not a fresh binary inspection or rerun. Recorded backend environment is pinning evidence, not independent instruction-level backend attestation.

All 30 owned invocations were exported from their isolated private homes, ten per home including negative gates:

- `data/runs/xps14-runtime-br37-owned-off-20261030/2026-10.jsonl`
- `data/runs/xps14-runtime-br37-owned-on-20261030/2026-10.jsonl` for `all3on`
- `data/runs/xps14-runtime-br37-owned-declarative-20261030/2026-10.jsonl`

Byte-identical export copies inside the evidence bundle make it portable without the rest of this repository. The exporter redacted these scratch runs as private. The unabridged diagnostic journals remain the runtime evidence, not the slim exports or lossy console rendering.

## Limits and separate correctness work

Cold support remains blocked by Blaze writable statics. Both Blaze stacks started from the same recorded br30 seed-store content, and these numbers are **seeded-warm only**. Comparing this filtered runtime loop does not establish full-suite, build, check, cold-start, or session-weighted performance. No significance or general claims follow from five samples, and no defaults are promoted.

Native correctness is separate, previously pushed in `89f61304b`. These latency observations do not replace its correctness gates, nor do they certify unrelated derive/attribute macro-cache experiments. No native/build/profile commands were run to retain or validate this evidence. Root FINDINGS, existing machine exports, harnesses, and pinned source were not edited by this publication.
