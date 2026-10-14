# October 14, 2026: reduced-category occupancy captures

This is profiling evidence, not a toolchain speedup comparison. See
`parallelism-map.md` for the corrected mathematical model.

## Experiment and provenance

- Same accepted west m8azn.6xlarge host, 24 reported processors, EPYC 9R05.
- Explicit inactive compiler `jr-b7907f1fe8f7`, unchanged default toolchain.
- Pinned Desktop snapshots: jcode-desktop `7d38fb7`, jcode `abff94eeb`.
- Separate `target-critpath-<compiler>-<scenario>` targets, not normal benchmark
  targets. Two warm runs and one profiled edit at each thread count.
- Profile categories: `generic-activity,query-blocked`. This removes query-key
  strings and several default event categories, but remains intrusive.
- Exact CLI SHA-256 is retained in `light-capture-provenance-20261014.json`.
  The launcher incorrectly supplied source HEAD as an executable revision.
  The collector explicitly marks that label unverified. The binary revision is
  unknown, rather than claiming it was compiled from that source HEAD.
- Five raw profile SHA-256s, two raw capture SHA-256s, wrapper SHA-256s and all
  fifteen result records are retained in the provenance bundle. Raw captures
  remain on the host and locally. Fifteen exact IDs are also committed as
  privacy-redacted exports under `data/runs/hosted-west-parallelism/`.

## What actually happened

| Public workflow | frontend threads | last unprofiled warm wall | profiled wall | noise caveat |
|---|---:|---:|---:|---|
| reference filtered tests | 1 | 4.535 s | 4.726 s | warm sample flagged noisy |
| reference filtered tests | 4 | 4.188 s | 5.024 s | profiled sample flagged noisy |
| reference filtered tests | 16 | 4.115 s | 6.665 s | both samples flagged noisy |
| body-edit check | 1 | 1.480 s | 1.563 s | neither sample flagged noisy |
| body-edit check | 16 | 1.467 s | 2.238 s | neither sample flagged noisy |

All nine reference runs reported three passing fps_counter tests, zero failing
tests. All six check runs exited successfully. The first run of each workload
was a cache-populating warmup (54 compiled units, 833 depcache hits), not an edit
latency sample. Subsequent runs rebuilt exactly one UI unit with 743 fresh units.
The check-only workflow confirms that the earlier mono/link emphasis does not
apply to every agent loop: its t=1 instrumented occupancy is dominated by macro
expansion, resolving, type checking and metadata, not linking.

Even the reduced-category t=16 check capture is much longer than its clean
unprofiled control. The reference controls also contain contamination flags.
Do not conclude that production parallel type checking or mono collection gets
slower from these captures. The five schema-2 summaries are localization tools,
not measured serial fractions or mathematical ceilings. No optimization was
promoted and no speedup is claimed.

## Requirement-to-observation mapping

| Requirement | Observation |
|---|---|
| Count nested events once | Fifteen focused analyzer regressions, including histogram/thread-time integral checks |
| Exclude reported query waiting | Enclosing-parent wait regression, parallel-worker and nested-wait regressions |
| Do not invent CPU work or a causal span | JSON null fields and real CLI/schema tests |
| Exercise two real workflows | Nine actual test runs and six actual check runs above |
| Preserve warm artifact reuse | Recorded one-unit rebuilds and 743 fresh units after initial warmup |
| Preserve noisy controls | All original noise flags retained in the collector output and cited table |
| Preserve raw evidence across sync | Real rsync fixture preserves nested profiles, JSON and partial markers while deleting stale source |
| Distinguish source from executable identity | Launcher now supplies only repository revision; executable revision remains unknown, SHA retained |
| Preserve hosted service defaults | No compiler activation, host resize, worker or permissions changes in this profiling window |

The Python benchmark suite passed 59 tests after the analyzer, provenance and
artifact-retention changes. A fresh audit verified both raw capture hashes, all
five raw profile hashes, all fifteen exact exported IDs, the five summaries'
histogram and thread-time integrals, and preservation of all five noisy runs.
This closes the profiling correctness loop, not the larger goal of achieving
more useful parallelism or a 10x compile improvement.
