# Diagnostic telemetry coverage

Telemetry must explain an agent's build result without collecting private source,
paths, crate names, test names, raw logs, argument values, environment values, or
credentials. Structured diagnostics remain best-effort and must never change the
build verdict, compiler environment, or artifact cache keys.

## Identity and attribution

Anonymous installation IDs are not identities. A `signed_in` boolean means the
client found credentials, not that the service authenticated an account. Login
flow joins can be absent when credentials were installed by another Jcode tool.
Only service-validated authentication may establish an account association.
An installation can be shared by several accounts. Do not guess a unique owner,
retroactively assign anonymous events, or treat absence of a login event as proof
that a person never signed in.

Queue authentication scope must be captured when an event is recorded. A later
login, logout, or account switch must not attribute older events to the new
account. Credentials must not be stored in telemetry queues or process arguments,
and custom telemetry endpoints must never receive hosted credentials.

## Diagnostic goals

- Correlate a finished run with its local `justrust show` and `justrust log` data.
- Distinguish cargo/compiler failures, failing tests, refused toolchains, recorder
  initialization failures, cargo exec failures, and routing fallbacks.
- Explain startup, lock wait, compile, build gaps, tests, and tail time.
- Explain dependency and workspace cache hit/miss counts and observed costs.
- Distinguish measured contention from incomplete CPU or compiler-phase coverage.
- Surface queued delivery, retryable failures, and rejected telemetry explicitly.
- Preserve anonymous telemetry and all existing opt-out controls.

## Structured schema

Schema version 2 adds validated run IDs, stable per-event UUIDs, explicit failure
and routing categories, pinned toolchain IDs, phase totals and build gaps,
compiler-pass coverage, CPU-accounting quality, test timing/filter counts, cache
miss/savings costs, and PSI availability. Recorder summaries also retain optional
sample/process counts, sampled time span and largest observation gap. Older
summaries omit observation coverage rather than pretending it was zero.

Client-only queue authentication markers are stripped before posting. The server
accepts only documented metrics and fixed labels. Stable event IDs deduplicate
retries within an install/account scope. Accepted prefixes are checkpointed, and
retry/authentication outcomes are visible through `justrust telemetry status`.
The nonblocking queue lock intentionally favors build latency over recording an
event during lock contention, so collection is not lossless.

## Limits that must remain explicit

An exact **system** compiler fingerprint is not currently in the run summary.
Pinned toolchain identity is available, but must not be represented as proof of
an unpinned executable's contents. Events rejected or interrupted before queue
append, abrupt termination, and internal shim panics can still leave gaps.
Deduplication is account-scoped, not global across a later account switch.

Structured summaries cannot reproduce a failing private test or reconstruct a
compiler diagnostic. Exact root cause can still require a user-provided log or a
separate, explicit redacted diagnostic upload. Do not silently turn usage
telemetry into raw log collection. An interrupted process may never reach its
finished-run event. Older client versions and previously anonymous events cannot
be repaired retroactively by a server migration.

Account attribution improves support and is not equivalent to anonymous usage.
Client notices and status output must accurately disclose this distinction.
A telemetry rollout is not complete merely because server tests pass: validate
queue scope changes, credential origin checks, server-side account derivation,
shared-install behavior, opt-out, real delivery, and unaffected cargo artifacts.
