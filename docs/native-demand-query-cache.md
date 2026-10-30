# Native demand-only query cache experiment

## Scope and status

`-Zjr-demand-query-cache` is a default-off, **TRACKED** boolean experiment.
It is a bounded experiment, not an architectural endpoint or a default
promotion. No speedup is claimed. Source readiness does not establish native
correctness or performance. Root coordinates native builds, source freezes,
and acceptance gates. Root's native semantic and mechanism gates passed on
2026-10-30, permitting the default-off experiment to be committed.

## Persistence behavior

With the flag off, persistence is unchanged. With it on,
`compiler/rustc_incremental/src/persist/save.rs` skips only the end-session
`tcx.dep_graph.exec_cache_promotions(tcx)` call. This avoids loading otherwise
unused green values solely to carry them into the next session's result cache.
The mmap is still closed at the same point and the existing
`on_disk_cache::OnDiskCache::serialize(tcx, encoder)` path still serializes
actually loaded or computed values under exactly its existing eligibility
rules. Dependency graph persistence and work-product persistence are unchanged.
There are no decoder, serialized-format, or verification changes.

The option is tracked, so changing it changes the tracked compiler options
fingerprint. On/off transitions must not be interpreted as same-configuration
warm-cache measurements. Acceptance must cover flag alternation as well as
repeated sessions within each configuration.

## Existing absent-value fallback

`compiler/rustc_query_impl/src/execution.rs` already distinguishes a green
dependency node from availability of its serialized value. In
`load_from_disk_or_invoke_provider_green`, a missing disk value takes the
`None` branch, calls the provider under `tcx.dep_graph.with_ignore`, and sets
`verify = true`. The existing `incremental_verify_ich` call then checks the
recomputed value against the previous fingerprint. This experiment does not
modify that fallback or its verification.

Skipping retention can move work into a future session. A green value not
demanded this session may no longer have a serialized value when a later edit
demands it, requiring provider execution and hash verification. Smaller current
persistence work therefore does not establish lower session-weighted wait.
Demand alternation and real multi-session timings are necessary to evaluate
that tradeoff, including potential regressions.

## Optional profile evidence

When incremental persistence reaches the skip branch, the GenericActivity
`jr_demand_query_cache_promotion_skipped` marks that branch. It reports neither
an avoided-value count nor avoided time. Its duration is not an estimate of
saved work. The normal `incr_comp_query_cache_promotion` activity remains inside
`exec_cache_promotions` and is not executed by this call when the flag is on.
An absent marker alone proves nothing if persistence did not run or profiling
was not enabled.

Existing `jr_incremental_green_recompute` activities, with query-name arguments,
can provide evidence that missing-value provider fallback occurred.
`jr_incremental_green_query` is broader and does not itself prove disk reuse or
recomputation. Profile evidence is optional and separate from mandatory semantic
acceptance. No synthetic avoided count is added.

## Root-owned acceptance before conclusions

- Native build and differential correctness against the unchanged configuration,
  including diagnostics, outputs, test behavior, and incremental verification.
- Repeated demand-changing sessions and flag alternation, exercising later
  demand for values omitted from an earlier session's cache.
- Real before/after session timings under identical workload settings, including
  future recomputation costs. Comprehensive session-weighted gates are required
  before any general speed claim or default promotion.

## Native acceptance outcome (2026-10-30)

Root ran `bench/reuse-demand-cache-accept.py` from harness commit `68d1002c0`
against its frozen native toolchain. The report records `passed` for semantic,
mechanism, and overall verdicts: all 32 revisions across nine modes (288 main
compiles, plus dependencies), with exact-byte diagnostics and runtime checks,
`-Zincremental-verify-ich`, persistent off/off and on/on replays, and tracked
off/on/off transitions. Ten separate mechanism processes passed the expected
normal-versus-skipped marker checks across cold, unchanged, narrowed demand,
unchanged narrowed demand, and restored wider demand. Green recomputation
counts remain observational, not a causal avoided-work claim.

Frozen driver SHA-256:
`ed21691a9db267ffb71cc3b8975a8cd89768397a7a7f82c9c839083485424efd`.
The report records before/after compiler and driver identities. Local evidence:
`/home/jeremy/.jcode/scratch/demand-query-cache-20261030/acceptance/acceptance.json`
and sibling `acceptance.log`. Root confirmed native source stayed unchanged
since the freeze. This worker did not run native builds or workloads.

These gates establish acceptance only for the exercised fixtures, not all Rust
programs or lower agent wait. Real timing and comprehensive session-weighted
performance gates remain root-owned and pending. The flag stays default-off,
with no performance claim and no default promotion.
