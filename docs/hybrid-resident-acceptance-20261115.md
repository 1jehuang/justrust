# Hybrid resident acceptance, 2026-11-15

Status: experimental and opt-in. Production defaults are unchanged. **Promotion
is blocked by a stale proc-macro result in the automatic-preparation matrix.**
This is an acceptance notebook, not a launch claim or authorization to promote.

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
