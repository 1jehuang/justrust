# Hosted cache reuse investigation, 2026-10-15

## What the counters mean

The production attributed-hosted telemetry subset contained 735 runs, 7,510
compiler unit invocations, 196,623 Cargo-fresh units, 2,973 dependency-cache hits
and 1,500 misses, and 53 workspace-cache hits and 2,504 misses. Counting fresh
units and explicit hits as avoided compilation gives approximately 97.8% observed
unit reuse. Dependency lookups hit 66.5%; workspace lookups hit 2.1%. These are
not interchangeable denominators, byte-weighted reuse, or compute-weighted reuse.
Telemetry coverage starts October 13, not the lifetime of the service.

The larger historical remote bucket mixes hosted and other remote runs and must
not be presented as all-hosted: 3,188 runs, 1,540,707 fresh units, 70,540 compiler
unit invocations, 41,132/17,436 dependency hits/misses and 679/9,982 workspace
hits/misses. Its reported cached compiler wall savings total approximately 6.25h.
The goal is lower end-to-end agent wait and compilation cost, not a maximized
lookup ratio that excludes Cargo-fresh units or ignores unit sizes.

## Changes

* Remove the workspace cache's two-entry same-source/different-environment cutoff.
  A third stable tracked environment variant is valid and now remains cacheable.
  Fresh numeric seconds/milliseconds timestamps still suppress admission. Truly
  changing nonnumeric build IDs can now incur extra background storage and
  eviction work, so no general performance claim is made for this fix.
* Add an experimental target-bound tier, enabled only by
  `JUSTRUST_LOCALCACHE_TARGET_BOUND=1`. Portable lookup remains first. Artifacts
  embedding runtime target paths can be reused only with the exact concrete
  target and profile paths. Manifest and output hashes have a separate versioned
  domain. Portable guards, legacy-stage refusal, dependency-cache behavior and
  stock fallback remain intact. This does not make artifacts cross-worktree safe.
* Forward the workspace-cache opt-out and experimental flag to hosted builds,
  without forwarding machine-local cache-directory overrides.
* Record target-bound outcomes in the finisher log. Existing historical refusal
  counts cannot be attributed retroactively to this opportunity.

## Validation and focused measurements

Final hosted check, full tests (600 passed, 3 ignored) and strict all-target
clippy passed: `20261014-175309399-3982529`,
`20261014-175314579-3984435`, `20261014-175331970-3989820`.
Python benchmark tests: 29 passed. Tests cover stable environment rotation,
fresh stamps, same-target reuse, cross-target/profile misses, runtime references
resolving the current executable, changed inputs, eviction, opt-out, and legacy
staging/namespace separation. A concurrent-fork ETXTBSY in the initial executable
fixture was addressed by running that fixture in an isolated test subprocess;
subsequent full suites passed.

Pinned `justrust-integration-retouch` measurements use the same f97fcc2 workload,
identical-content source retouch, and local execution as a controlled benchmark
exception. Results and executable hashes are in:

* `bench/results/20261015-target-cache-before.json`: 0/3 workspace hits.
* `bench/results/20261015-target-cache-optin.json`: 5/5 workspace hits.

All samples passed ten tests. `bench/suite.py compare` reports median 0.77s to
0.49s, delta -0.28s (-37%), **uncontrolled (noisy or unknown samples)**. This is
not a demonstrated general speedup. It does demonstrate avoided recompilation
for this specific workload. Other agents were building concurrently.

Real hosted retouch/reuse runs `20261014-175607565-4021656` and
`20261014-175613844-4021580` passed 22 integration tests each, with one workspace
cache hit on the second run. Hosted opt-out run
`20261014-175727124-4051337` compiled without workspace-cache counters, and
`20261014-175731629-4052571` then reused all 62 Cargo-fresh units, confirming
that toggling admission/restoration did not invalidate build identities.

Desktop's real one-line FPS test edit and exact backup restoration passed three
tests each: `20261014-175634245-4031767` and
`20261014-175707182-4043732`. Its timestamp-dependent UI compilation remains a
legitimate miss. Concurrent unrelated source edits prevent timing attribution.

## Broad gate and next steps

An additional raw SSH unrecorded-build comparison is inconclusive. Its first
attempt used a different Cargo home and correctly invalidated source paths;
with matching Cargo home the raw invocation aborted with
`assertion failed: output.write(&bytes).is_ok()`. The normal hosted recorded
invocation immediately before it passed 22 tests. This extra probe does not
establish recorded/unrecorded artifact equivalence and needs investigation.

The comprehensive seed-1, n=1 baseline was attempted with a pinned original
binary, but exceeded the 600-second task deadline before completing all 40
points. Its paired candidate was cancelled rather than publishing an incomplete
comparison. The new tier stays opt-in. No default promotion or overall hosted
speedup is claimed. Repeat the broad gate with a sufficiently long deadline,
including nonnumeric environment churn and background copy/hash/GC contention.

Next instrumentation should report refusal reasons, committed bytes, finisher
CPU/wall cost, eviction and reuse distance. Genuine new edits require finer-grain
native compiler/query reuse rather than pretending semantically different
outputs are cache hits. Architectural replacement is appropriate when measured
agent wait improves while Rust semantics, runtime paths and tenant isolation
remain correct.
