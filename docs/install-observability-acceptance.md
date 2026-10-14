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
