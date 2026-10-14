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
  with unknown outcome, not success. Independent production queries at actual
  wall-clock time on 2026-10-14 at 12:06:31 UTC through 12:07:05 UTC confirmed
  `unknown_lost`, still with no summary or terminal event. This is observed
  expiry, not only a synthetic future-time test.
  The privacy-safe interrupted observation is recorded separately under
  `data/runs/xps14-install-observability/observations/`, rather than fabricating
  a finished-run summary.
- Normal authorized and billed hosted check
  `20261014-044551276-2781317` completed successfully with returned artifacts.
  Initial terminal telemetry delivery failed and remained in a same-scope local
  queue with status `retry`. An explicit same-environment background retry at
  11:51:01 UTC changed delivery status to accepted and drained the retained
  queue. Initial compile success alone was not treated as proof of terminal
  telemetry delivery. After the bounded detached retry improvement, fresh normal
  billed hosted run `20261014-045550863-2888689` automatically delivered trusted
  queued/running/summary/done events with exit 0 and `local_only` logs. Its server
  ledger independently agreed `done`, exit 0. No manual flush was used for this
  fresh run. Local delivery health recorded HTTP 200 and curl exit 0.
- The original delayed hosted completion exposed a separate stale-ledger gap:
  trusted client telemetry said done while the server ledger became lost. Cloud
  source `29af27f`, deployed version `7b61a917-3821-4b79-93e6-03bf5bd1c865`,
  now reconciles only an already existing same-account hosted row from an exact
  accepted stable-ID remote completion. It preserves explicit terminal rows and
  never creates a charged run or changes billing. The full cloud gate passed
  247 JavaScript, 23 Python, and 17 dashboard tests.
  Replaying the original accepted event, without changing UUID, observation time,
  or data, changed that old ledger to done, exit 0. An independent query confirmed
  its original queued time and single stored done-event row were preserved.
  A second exact replay was accepted idempotently. Authoritative account checks
  before/after matched the original owner, and all three compared build-counter
  fields were present and unchanged. Ledger completion receipt time is not a
  claim that the original compile ran until the delayed replay.
- Actual disk-limit run `20261014-051059525-3009368` applied a 64 KiB file limit
  to the synthetic recorder with the resident daemon disabled. Journal and staging
  writes failed, but Cargo/test exit remained 0 and the final stdout marker was
  preserved. Production accepted the bounded `recording_persist` / `log` failure
  with actual exit 0, and done reported exit 0 with no complete log claimed.
  Raw error text, file paths, and synthetic log content were absent from that
  diagnostic payload.

## Final validation and rollout boundary

- Final source integration: `68b20fe` persistence diagnostics and `2dd8f34`
  isolated strict-fixture telemetry/update opt-outs. `cargo fmt --check`,
  `justrust check`, `justrust test`, and strict all-target clippy passed. Tests
  reported 437 passed, 0 failed, 3 ignored on the first pass. Runs are
  `20261014-050608672-2967094`, `20261014-050619264-2972060`, and
  `20261014-050712859-2984510`. These plain invocations routed locally because
  of the recent-remote-failure cooldown, rather than being mislabeled hosted.
  Explicit root hosted check `20261014-051329269-3025066` also passed normally.
  `cargo install --path . --locked` installed the settled client.
- Website installer source `c48b57a` passed 48 installer/Pages acceptance tests
  and 37 beacon/layout tests. Actual Pages workflow `38559761963` deployed it.
  Live artifact equality, response privacy headers, token-free URL rendering,
  default-origin credential guards, and parent-shell override handling were
  verified without minting a production user's token or key.
- Backend source `7151e33` and cloud services are deployed. The client is
  installed on the development machine. Public alpha.18 publication remains
  gated on the final exact-source native platform workflow, including both
  macOS runners. Linux, Windows, and MSRV passed for source `2dd8f34`, but queued
  macOS jobs are not treated as passing native macOS validation. Installer macOS
  path mocks do not substitute for that gate.

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
