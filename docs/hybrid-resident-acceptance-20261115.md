# Hybrid resident acceptance, 2026-11-15

Status: experimental and opt-in. Production defaults are unchanged. **Promotion
is blocked pending reliable managed timing, a valid broad comparison, and safe
handling of the remaining compile-time input boundary below.** The final automatic
matrix now passes 37/37 cases. The targeted macro freshness regressions now
pass on v5. This is an acceptance notebook, not a launch claim or authorization
to promote.

## Latest accepted focused results: native v5, 15:28 UTC

The later `p4` run supersedes the earlier p3 timing diagnostic below. It passed
**25/25 strict correctness rows**, including phase-specific error/recovery route
checks, five paired edits, and actual LRU eviction. The final automatic `a5`
run with client v6 and engine v5 passed **37/37 rows**, including compile-time
macro freshness, compile-only preparation, exactly-once execution and cleanup.
The unsupported automatic-check path now emits an explicit skip receipt without
starting preparation. All raw receipts and hashes are in
`bench/results/hybrid-resident-v5-final-20261115.tar.gz`.

| Operation | Published Cargo control | Ready-only hybrid |
| --- | ---: | ---: |
| Body-edit median, five samples | 3.9577 s | 0.1231 s, resident committed |
| After proven eviction, one paired sample | 4.0730 s | 4.0747 s, Cargo fallback |

That is an observed **32.2x** ratio of hot-edit medians on this narrow fixture,
not an end-to-end managed speedup or an isolated measurement of RAM residency.
The median of the five paired speed ratios is also **32.15x**, with individual
pairs ranging from **28.01x to 33.04x**. This is only four selected tests from one
pinned package, not the full project or the 40-point broad workload suite.
The host was not demonstrated quiet, and a client build overlapped part of the
run. Both compiler/backend and retained state differ from the control.
Manual initial preparation cost **37.574 s**, excluded from the hot median.
At the observed median saving, its illustrative compute break-even is about
**10 hot edits**, not a measured amortization guarantee. Peer preparation cost
another **13.142 s**. The namespace used approximately **4.09 GiB PSS** just
after preparation and **3.49 GiB** before peer loading. After proven eviction,
the retained peer namespace used **2.15 GiB PSS**. These are sampled observations,
not peak memory or a hard memory limit. The 1 MiB setting exercises a soft LRU
policy, not bounded heap restoration.

Current frozen client v6 SHA-256:
`df496033304ecd59e980b7ed2b3d164bca5ecc5d8522c0aadd81e590cffe7289`.
Current frozen engine v5 SHA-256:
`980535cc07d248cb2a255a69569af288e26e85383889f0bf1015279bc8b0ecb3`.
Earlier artifact identifiers and runs below are retained as historical evidence.

## Same-client managed follow-up: no timing acceptance

The v7 follow-up used the exact same frozen client v6 for ordinary Cargo and
hybrid requests, eliminating our deliberate published-versus-candidate daemon
version alternation. It did not finish: authoritative guest-side-effect reads
failed after three read-only retries with normal hosted admission reporting
`Hosted builds are temporarily unavailable`. The original report remains failed.
No speedup is inferred from this incomplete run. A subsequent normal admitted
shutdown stopped only its private warm namespace; independent exact-namespace
process probes returned `[]` for both cold and warm homes. All 48 archived
receipt hashes were verified in
`bench/results/hybrid-managed-hosted-same-client-failed-20261115.tar.gz`.

The preceding managed v6 experiment's 149 original receipt hashes were also
verified in `bench/results/hybrid-managed-hosted-20261115.tar.gz`. Its initial
cleanup failure is preserved separately from the later independent empty probe.

## Completed broad comparison: promotion rejected

The immutable candidate finished **all 40 canonical points** at 16:23:35 UTC,
after **1,995.344 seconds**. This completed after the requested three-hour work
minimum instead of cutting the workload short. Final input-hash verification
passed before any future generator-control changes.

`compare --require-valid` returned **exit 2**, correctly blocking promotion:

- Baseline: three unexpected failure points (zero-based 28, 35, 36) and eight
  noisy points. Desktop fixtures lacked Git author identity under isolated HOME.
- Candidate: four unexpected failure points (4, 28, 35, 36) and 31 noisy points.
  Point 4 failed a TUI image-label assertion while on ordinary Cargo fallback.
- Actual outcome-parity coverage was invalid at those four points. No failure
  or noise gate was waived.
- Every one of the **36 retained measured successful-sample logs** reported
  Cargo fallback. **Zero measured samples committed to native execution.**
  Some separate preparations succeeded, and others exposed unsupported macro
  compilation or panics. This does not establish a single cause for every miss.
- The sidecar retained all **85 raw logs**, with no reported overflow or missed
  logs. The exact private local resident namespace shut down successfully
  (`daemon: stopped`); the sidecar exited successfully. No owned hosted jobs
  remain. No further benchmark runs are scheduled in this session.

Consequently there is **no accepted broad speedup or native-coverage result**,
and production defaults remain unchanged. The focused 32x result above must not
be generalized to this workload. Final raw receipt paths and hashes are retained
in `bench/results/hybrid-canonical-20261115.tar.gz` and
`docs/results/hybrid-canonical-20261115.json`. Root independently verified the
archive SHA-256, all **185 original retained log hashes**, and the three primary
report hashes before closing the gate.

After final source-hash verification and comparison completed, `8bc8e59e1`
added a **future-run-only**, explicit `--fixture-git-identity` adapter control.
It requires the isolated XDG fixture policy and gives both stacks the same
literal benchmark-only author/committer identity without editing global Git
configuration. Three added tests include real `git init`/`git commit` through
both isolated adapters. Root's final hybrid Python discovery passed **72 tests**.
No fresh canonical pair was run with this option, and the failed original pair
was not rewritten or reclassified.

## Follow-through hardening after the frozen performance candidate

These changes are separately tested and are **not** part of the frozen v6
performance candidate. No new speedup is attributed to them:

- `8e0be946d`: sync-daemon replacement waits for the retiring daemon's lock,
  rather than treating socket EOF/read timeout as proof that manifest saving
  finished. A real startup failure showed the old daemon saving for 10.27 s
  after shutdown while the replacement exited on its held lock. The new bounded
  handoff and concurrent replacement reuse passed two deterministic regressions,
  seven hybrid library tests, hosted check and clippy.
- `6270e1e57`: reject impossible Unix daemon socket path lengths by raw platform
  byte capacity, before engine launch. Boundary, multibyte/non-UTF-8 and
  no-engine-spawn subprocess checks passed. The correct library unit filter
  passed three tests; CLI routing passed 14 and deferred preparation passed 12.
- `3e7a68ebd`: automatic preparation failure cooldown. Under the existing namespace lock,
  record a failed engine invocation and suppress additional automatic engine
  attempts for 60 seconds. Manual preparation and foreground routes are
  unchanged. Hosted check, three library tests, **15 deferred-preparation
  subprocess tests**, 14 routing tests and clippy passed. The synthetic engine
  validates suppression, expiration, recovery and unsafe marker rejection, not
  native compatibility or performance. The running canonical benchmark still
  uses the old frozen client without this cooldown.
- The final hybrid Python harness/receipt regression discovery passed **69 tests**.

All changes remain opt-in or unrelated startup correctness fixes, with the
installed production CLI unchanged. Passing these focused gates does not waive
the broad comparison or compile-time input requirements.

## Next promotion gates, ordered by observed leverage

1. Make ready admission safe for **all compile-time read inputs**, or reject
   unsupported input classes before commitment. The current tracked local macro
   source closure does not cover arbitrary external include/build-script data.
2. Establish native coverage on the real broad workloads. The frozen canonical
   candidate currently fails native preparation for `const-random-macro 0.1.16`
   through `getrandom::imp::getrandom_inner` with `UnresolvedMethod("map_err")`.
   Cargo fallback is not evidence that native compilation supports that workload.
   Repeated automatic preparation failures are a separate scheduling cost.
3. Repair the **symmetric benchmark fixture environment** and rerun unchanged
   broad gates. The isolated baseline lacked Git author identity for desktop fixtures. A
   symmetric future adapter control now exists, but it has not yet been used in
   a fresh full comparison. The candidate also recorded a TUI assertion mismatch
   while on Cargo fallback. Keep those failures and the noise gates, rather than treating
   an invalid comparison as a promotion result.
4. Rerun same-client ordinary managed Cargo versus resident routing without
   sync-daemon version churn or admission outages, retaining preparation,
   exactly-once effects, cleanup, and paired timings. The narrower guest-shell
   benefit does not establish this end-to-end result.
5. Measure background-preparation interference, useful-edit amortization and
   aggregate multi-user memory pressure before enabling automatic preparation
   or retention defaults. A soft LRU setting is not a tenant RAM limit.

## Frozen artifacts

- Client: release-mode `justrust-v5-release`, SHA-256
  `0197a3e67c2e4382b82c56d9f247004e11c7518120479dec0772baee54d3785a`.
- Native engine: `jr-engine-v3`, SHA-256
  `719678f307013168660019ebb0da174052fa000f2ebc0182f481cce9b1d620e8`.
- Guest published control: SHA-256
  `89918bc210f184ade36bd7109249d87adab0c4036e6b3a0a5b5cdae5f5f2b115`.
- Native compiler: explicit `nightly-2026-10-27`, built on the authenticated
  account's hosted guest. The experimental engine remains Blaze-derived and
  uses Cranelift. These measurements do not isolate RAM residency from backend
  and compiler changes.

No candidate was installed over the production CLI.

## Correctness evidence

The exact pinned native build passed `justrust check`, all **407** binary unit
tests, release build, and **21/21** real private-decision protocol tests.
The latter include malformed syntax, type errors, assertion failure, recovery,
legitimate zero-test filters, configuration and path-dependency changes,
artifact-directory isolation, compile-only preparation and exactly-once effects.

Real testing found and fixed these bugs rather than relaxing acceptance:

- Stale configuration and external path-manifest readiness identity.
- Build-script reads considered unchanged when size and timestamp matched even
  though file contents differed.
- Required engine namespace missing inside sandboxed compiler probes.
- Malformed crate roots accepted as standalone expression includes, producing
  success with zero tests instead of a syntax error.
- Background preparation lowering future foreground workers' scheduling priority.

The generated release-mode differential matrix (`g3`) passed **37/37 rows**,
including invalid syntax returning 101 with no user execution on both stacks.
Tools were unchanged and exact private resident namespaces were cleaned up.

A real managed `__agent` cancellation fixture passed against the frozen v2 and
v3 engines: the long-running native test executed once, cancellation returned
130, its actual process disappeared and the same resident daemon handled a
subsequent successful request. This does not prove cancellation of a compiler
worker after the client has committed and then disconnected.

## Focused bounded-memory diagnostic

Fixture: pinned `f97fcc2`, `summary::tests::`, four tests per successful request,
with fresh runtime nonce and append-only exactly-once side-effect witnesses.
Five rotated body-edit samples per stack. These are **guest-shell subprocess
latencies**, not laptop-to-host managed request latencies.

The host was heavily contended: reported one-minute load was approximately
44 to 53 on a 16-vCPU guest. Other accepted experiments also overlapped part of
this run. Treat the following as a contended diagnostic, not a quiet-host
performance guarantee or broad promotion result.

| Operation | Published control | Hybrid |
| --- | ---: | ---: |
| Fresh fixture request, one sample | 12.848 s | 11.377 s, Cargo fallback |
| Body-edit median, five samples | 5.826 s | 0.601 s, resident committed |
| After proven native LRU eviction, one paired sample | 6.219 s | 6.166 s, Cargo fallback |

The observed body-edit ratio is approximately **9.7x** on this fixture and run.
The important eviction result is that the hybrid selected ordinary Cargo rather
than paying for a cold native rebuild. It does not establish a universal
worst-case latency or prove zero routing overhead.

Manual native preparation took **70.847 s**, excluded from the hot-edit median.
Preparing a peer to force eviction took another **38.017 s**, also separately
recorded. Preparation is not free and must be amortized over subsequent edits.
At this run's approximately 5.224-second median saving per hot edit, the
70.847-second initial preparation corresponds to roughly **14 subsequent hot
edits** before its compute-time cost is recovered. This is illustrative
arithmetic, not a measured break-even workload: background preparation may
interfere with foreground work, eviction can erase the investment, and the
host was contended. Off the critical path does not mean free.

The budget test proves a fresh LRU eviction log, the original worker's exact
PID/start-tick disappearance and retention of the original daemon. Namespace
PSS was approximately **3.49 GiB** before loading the peer and **2.06 GiB** after
eviction, with only the peer worker retained. These are point-in-time process
PSS observations for this sequence, not a general memory-saving percentage.
The configured 1 MiB budget deliberately forces the soft one-worker retention
policy. It is **not** a hard memory cap: a single hot compiler can exceed it.

Receipt: `p3/report.json` inside
`bench/results/hybrid-resident-focused-20261115.tar.gz`, alongside the generated
matrix and original root-level logs. Error/recovery commands passed differential output,
exit-status and side-effect contracts. Their route labels need the stricter
phase-specific acceptance follow-up, so this notebook does not yet count those
rows as additional proof of committed-route error handling.

## Automatic preparation found a correctness blocker

The real `a3` automatic-preparation workload exposed a stale proc-macro result:
after changing the macro's emitted value from 7 to 11, published Cargo passed
with 11, while the resident-committed hybrid still emitted 7 and failed its
assertion. The user test executed once, so exactly-once dispatch alone does not
establish compiler correctness. The manual generated matrix had not exposed
this because an intervening cold fallback masked retained-worker invalidation.

The engine must either invalidate these compile-time inputs before commitment
and safely fall back, or correctly reload the macro and its dependencies. The
strict differential gate remains unchanged. No promotion is justified until a
fresh actual automatic run passes this case and recovery.

The first remediation checkpoint, engine `jr-engine-v4` (SHA-256
`4faac0600ce11992aa23b3e98a1d64bfa8cc79cd9ab664e4b7a5feceed2f5c13`),
passed check, release build and all **412** full binary unit tests. It correctly
missed before commitment after same-size/same-timestamp macro and helper-dependency
changes. However, **2 of 24** actual protocol cases still failed: explicit
preparation followed by a newly committed request reused the old macro value.
A readiness key alone does not invalidate all underlying compiler artifact or
proc-macro-server state. The stricter rewarm regression remains failing and is
not waived.

Separately, this exact v4 artifact passed the real postcommit compiler-disconnect
acceptance: an in-flight owned compiler was reaped, no request was replayed, and
the same daemon successfully prepared and handled a later request. This is
additional lifecycle evidence, not a resolution of the macro freshness blocker.

The next checkpoint `jr-engine-v5` (SHA-256
`980535cc07d248cb2a255a69569af288e26e85383889f0bf1015279bc8b0ecb3`)
uses separate retained workers and immutable proc-macro artifact namespaces for
changed compile-time input generations. It passed the exact pinned check,
**413/413** full binary unit tests, release build, **24/24** unchanged real
protocol cases, and the real compiler-disconnect case. Both previously failing
same-size/same-timestamp macro and local-helper edits now safely miss, published
Cargo sees 11, and explicit rewarming is followed by a successful committed
request that also sees 11. Ordinary application source edits with a proc macro
still use the ready worker.

Receipt: `bench/results/hybrid-resident-v5-acceptance-20261115.tar.gz`.
This closes the targeted reproduction, not the broader automatic-preparation,
managed-latency, or canonical comparison gates. Separately, the current hybrid
Python harness suite passed **64** tests and the integrated state-restore,
resident-tier, runtime-context and comparison-gate suite passed **82** tests.
These are harness regression results, not additional native performance samples.

### Remaining compile-time input boundary

The targeted fix is not a complete compile-time dependency tracker. The current
precommit key does not cover macro include data or build-script reads outside
the tracked local package roots. Nested `target`/`.git` trees and mutable
registry/git source contents are also outside that package-content key.
Downstream caches containing read facts do not by themselves prove that ready
admission will reject every such change. These remain correctness limitations
of the opt-in experiment, even if a particular broad workload passes. General
Rust parity and unrestricted default promotion are not established.

## Actual managed hosted diagnostic, client v6 / engine v5

The fresh `managed-v6` attempt completed **30/30** foreground contracts through
ordinary `justrust test --remote`, including five rotated body edits per route,
type errors, assertion failures, recovery, exact route receipts, four-test
selection parity and exactly-once remote side effects. Tools were unchanged.
Manual preparation succeeded in **39.188 s** including its management request,
separate from foreground timings.

| Managed body-edit route | Median, five samples |
| --- | ---: |
| Published shipping control | 6.088 s |
| Hybrid with no resident worker, Cargo fallback | 2.242 s |
| Hybrid with ready resident worker, committed | 1.879 s |

Transport/setup latency varied substantially. The ratio of shipping and warm
medians is 3.24x, but the **median of the five paired shipping/warm ratios is
only 1.52x** (individual ratios 10.57x, 1.45x, 1.58x, 0.90x and 1.52x).
The cold hybrid also sometimes beat shipping despite using Cargo. This is a
small variable-latency diagnostic, not evidence of a universal 3.24x advantage,
and not comparable to a quiet in-process compiler-only benchmark.

Inspection of the real local sync-daemon log identified a specific confound:
alternating the published and candidate executables repeatedly changed the
global daemon build identity, triggering shutdown, reconnect and mirror restore.
Other agents' client builds also caused replacement. Therefore this is **not a
clean estimate of the resident compiler's managed speedup**. A separate fresh
same-client hybrid-OFF versus hybrid-ON experiment is required to avoid inducing
that version churn ourselves. The observed daemon startup failure also exposed
a bounded shutdown-lock handoff race; its fix is separate from these frozen
measurements.

The run's **overall gate failed initial cleanup**: both stop commands returned
zero, but one process still carried the cold namespace marker at the 30-second
probe deadline. The warm namespace was empty. A later independent normal
admission probe verified both exact namespaces empty, without killing any
process or altering the original failed report. All foreground measurements
remain raw evidence, but the lifecycle deadline failure is not waived for
promotion. The preceding `managed-v5` attempt failed at admission with a 503
before its first warm edit and is retained separately in the same archive.

Receipt: `bench/results/hybrid-managed-hosted-20261115.tar.gz`, **149** original
receipt hashes verified. Includes original failed overall reports and the
independent `cleanup-followup.json`; no source, target trees or environment
dumps are archived.

## Managed hosted path and remaining gates

The real `justrust test --remote` benchmark uses ordinary subscription admission,
source synchronization and the managed child lifecycle. A guest-shell speedup
must not be substituted for this end-to-end result.

- First managed attempt produced correct tests and exactly-once effects, but
  the intended warm request fell back. Manual preparation had warmed a different
  PATH identity. The harness now matches the agent's PATH prefix and explicit
  mirrored Cargo home, with regression coverage.
- The next attempt was blocked by native TLS EOF and transient hosted admission
  503 errors. No speedup is claimed for either attempt. Private namespaces are
  stopped through lifecycle commands and separately checked after recovery.
  Both interrupted v2 namespaces subsequently reported no running daemon and
  zero matching resident processes through normal authenticated admission.
- A valid matched canonical 40-point session-weighted comparison is still
  required, including real committed native coverage. An all-fallback run only
  establishes router compatibility, not the benefit of resident compilation.
- Automatic-preparation interference, transport-level postcommit compiler
  cancellation on normal managed requests, and stable managed-request timing
  remain separate acceptance work.

Full decoded compiler-heap disk snapshots are not implemented or measured here.
