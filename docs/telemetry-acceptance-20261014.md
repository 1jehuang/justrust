# Diagnostic telemetry acceptance, October 14, 2026

## Implemented

- Optional authenticated telemetry resolves accounts through the subscription
  backend. Anonymous clients remain supported. An invalid supplied credential
  never silently downgrades to anonymous attribution.
- Queue events carry local credential-scope markers. Old unscoped events and
  events recorded under a different credential remain anonymous. Markers and
  credentials never enter telemetry JSON. Hosted credentials go only to the
  exact trusted default origin, through curl stdin rather than process argv.
- Stable event UUIDs and account-scoped server deduplication prevent same-scope
  retries from double counting. The client checkpoints accepted prefixes.
- Explicit schema/run IDs, pinned toolchain identity, routing and failure
  categories, build gaps/phase totals, CPU accounting quality, missing compiler
  pass coverage, cache miss/saved time, test timing/filter counts and PSI quality.
- Optional sample/process counts, sampled span and largest observation gap.
  Old summaries omit these fields instead of reporting invented zeroes.
- Local recorder/toolchain/exec and remote setup/admission/transport/artifact/
  summary failures have structured diagnostics without private error text.
- The server allowlists metrics and fixed labels. Diagnostic tools distinguish
  trusted account links, ambiguous shared installations and sign-in evidence
  without a linked login flow.

## Validation

Final `justrust check`, full `justrust test` (427 passed, 3 ignored), and
`justrust clippy --all-targets -- -D warnings` passed. `cargo install --path .
--locked` installed the completed client.

| Check | Run / evidence | Observed result |
| --- | --- | --- |
| Full tests | `20261014-040509911-2371434` | 427 passed, 0 failed, 3 ignored |
| Final clippy | `20261014-040537632-2382766` | Warning-free |
| Recorded/unrecorded artifact sharing | `20261014-040142704-2318604`, `20261014-040143226-2318732` | Unrecorded Cargo said Fresh, next recorded check compiled zero units |
| Required toolchain refusal | Isolated fixture queue | Exit 1 unchanged, exactly one `toolchain_required` event |
| Opt-out | `20261014-040143596-2318864` | Unsent events deleted, subsequent build did not enqueue |
| Observation coverage | `20261014-040657767-2399079` | Actual test passed, 2 samples / 1 process / 0.18 s sampled span and largest gap |
| Actual client authenticated upload | `20261014-040753300-2405164` | Production D1 has trusted run event, stable UUID, four observation metrics and one install/account link |
| Restored hosted build path | `20261014-040706043-2400213` | Normal charged hosted check passed; compiler 6.2 s, client 60.5 s including 53.6 s sync |

All cited run summaries are committed under the task-specific
`data/runs/xps14-telemetry-*` shards. These are correctness checks, not a speedup
comparison. Intentional local acceptance isolates local artifact sharing and
opt-out behavior. The hosted check used normal authentication, admission and
billing.

Client queue acceptance also exercised actual installed execution against an
isolated fake curl transport: 150 accepted events followed by 2 retrying events
produced batch sizes `[150, 2, 2]`, not a replay of the accepted prefix. Legacy
queue and account-switch events stayed anonymous, custom endpoints received no
hosted credentials, argv contained no secret, and local scope markers were
stripped. Same-scope retry IDs remained stable.

Cloud tests passed: 238 Node, 15 funnel, and 17 dashboard tests. Production
anonymous replay retained one event, invalid auth returned 401 after backend
recovery, valid auth and replay returned 200, and trusted linkage was verified.
Historical pre-rollout anonymous events remained unattributed.

## Operational follow-through

Live acceptance discovered that existing backend routes had disappeared after a
stale deployment. Recovery restored the previously deployed route implementation
without bypassing billing or weakening tenant isolation. Backend tests passed
and normal hosted build admission was verified afterward. Cloud telemetry and
schema changes are deployed. Client distribution requires a new verified
release; a source install on this machine alone does not upgrade Jonathan's Mac.

## Remaining limitations

No private source, full error chains, test names, argument/environment values,
paths or raw logs are uploaded. Exact root-cause analysis of a private failure
can still require an explicit user-provided diagnostic log. System compiler
fingerprints, abrupt termination before queue append, internal shim panics and
nonblocking queue-lock contention remain gaps. Deduplication is scoped to the
observed account, not a global identity across credential switches. Existing
historical anonymous events cannot be attributed retroactively with certainty.
