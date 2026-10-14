# Installation-to-compile observability acceptance

This is the acceptance gate for the token-bearing installer audit begun on
2026-10-14. Pending checks are requirements, not claims of deployed coverage.

## Identity evidence chain

1. Browser session authorizes minting a short-lived, single-use install token.
2. Installer redeems that token without placing the token or returned key in
   child-process arguments, telemetry, diagnostic output, or retained fixtures.
3. With explicit telemetry consent, redemption associates a validated installation
   ID with the server-derived account. Consumption is not proof of local key
   persistence or a completed installation.
4. Successful local credential persistence emits an authenticated installer
   outcome. Failed persistence remains a failure, even after token consumption.
5. Existing-credential installation takes a separate verified path. A local key
   file alone is never described as server-verified authentication.
6. Compile events authenticate within their captured credential scope. Shared
   installation IDs can have multiple trusted accounts, never one guessed owner.

## Required installation cases

- Fresh token install and first successful local compile.
- Expired token, replayed token, and malformed token.
- Credential write failure after a successful exchange.
- Existing valid credentials and existing invalid credentials.
- Installer failure before binary persistence and login failure afterward.
- Reinstallation without unintended installation-ID replacement.
- Environment and persisted telemetry opt-out, including token exchange linkage.
- Linux and macOS path behavior, without minting a real user's key for testing.
- Website request URL, copied command, process argv, shell tracing, and telemetry
  payload review for token exposure. Tests use synthetic secrets only.

## Required compile cases

- Queued/running/terminal states correlate by validated run ID and trusted account.
- Successful build and compiler/test failure preserve actual exit status.
- Local and hosted execution report truthful routes and log availability.
- Interruption becomes stale/unknown, never inferred successful completion.
- Delayed and out-of-order heartbeats cannot overwrite a terminal state.
- Event observation time and server receipt time remain distinguishable.
- Offline queue retry, credential switch, and anonymous mixed batches do not
  misattribute events or block unrelated accepted telemetry.
- Network delivery is asynchronous and recording remains fail-open. Recording
  must not change artifact identity, compiler environment, or build results.

## Observed evidence, 2026-10-14

- Backend source `7151e33`, deployed version
  `71a2227d-03f1-436b-a042-0d998c3e31a1`, adds consented installation linkage
  to the existing atomic token exchange. The full backend suite passed 453 tests.
  Production migration invariants reported zero invalid linkage rows. Existing
  authenticated `/v1/me` remained available with hosted builds enabled.
- Cloud version `057bf7b0-7a17-40d1-9514-9fb19cf6546b` accepts authenticated
  installer outcomes and lifecycle events. Its validation passed 242 JavaScript
  and 36 Python tests. Anonymous lifecycle events were skipped without rejecting
  an otherwise valid batch, and invalid authentication returned 401.
- Installed lifecycle client source `41b787c` completed actual run
  `20261014-043509383-2672389` at 11:35:09 UTC. Production storage contained
  trusted queued, running, finished-summary, and done events for that same run.
  The summary reported three passing tests, no failing tests, and exit 0. The
  terminal event reported `local_only`, with output persisted on the client.
  All events occurred in the same second, exercising the ordering boundary.

- The installed integrated journal client completed actual success run
  `20261014-044251653-2732964` and deliberate failing-test run
  `20261014-044251789-2733020`, preserving exits 0 and 101. Production storage
  contained trusted queued/running/summary/done records for both, with terminal
  logs reported `local_only`.
- Actual interrupted run `20261014-044251870-2733060` emitted a heartbeat after
  30 seconds, then its recording process was killed with SIGKILL. The exact
  `justrust log` command recovered the synthetic output marker. There was no
  summary or terminal telemetry event. Production projection reported running
  with unknown outcome, not success. A deterministic expiry test reported
  `unknown_lost`; this does not claim that wall-clock expiry had yet occurred.
  The privacy-safe interrupted observation is recorded separately under
  `data/runs/xps14-install-observability/observations/`, rather than fabricating
  a finished-run summary.
- Normal authorized and billed hosted check
  `20261014-044551276-2781317` completed successfully with returned artifacts.
  Initial terminal telemetry delivery failed and remained in a same-scope local
  queue with status `retry`. An explicit same-environment background retry at
  11:51:01 UTC changed delivery status to accepted and drained the retained
  queue. Initial compile success alone was not treated as proof of terminal
  telemetry delivery. A bounded detached retry improvement and a new automatic
  hosted-delivery acceptance check remain required before final freeze.

The guarded public installer live deployment, final settled source validation,
and exact-source native release checks remain separate acceptance gates.

## Delivery and identity limits

Installer reports are capped at three seconds and are best-effort, especially
before a binary exists. Durable consented token consumption is not proof of
successful client credential persistence. Missing client outcomes stay unknown.
An exported `JCODE_API_KEY` in the calling shell can override a newly persisted
key on future builds. Installation identity therefore never substitutes for
service-verified per-event account identity. Remote fallback attempts currently
have separate run IDs without a trustworthy attempt-link field.

Local journal persistence has a nominal one-second tick, plus scheduling and I/O
delay, and does not claim fsync or power-loss durability. A killed process can
lose its most recent unflushed output. Failed persistence preserves the compiler
verdict and must be distinguished from a compiler failure. No collection path
claims lossless recording or retroactive attribution of old anonymous events.

## Logs and support limits

`local_only` means the client has a local log, not that an operator can read it.
`none` means no persisted log is claimed. Uploaded availability must not be
accepted until a real authenticated, explicitly consented upload exists with
retention, access controls, and secret handling verified end to end.

Raw logs can contain private source, file paths, argument values, and credentials.
Structured telemetry does not replace them or justify silently collecting them.
A support investigation must identify the exact run and available local log,
then obtain a user-approved diagnostic artifact when structured evidence is
insufficient. Missing evidence, old anonymous records, and old client versions
remain explicit limitations rather than guessed attribution.
