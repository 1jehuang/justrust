# Candidate-free native Cargo execution in the build daemon

`JUSTRUST_BUILDD_NATIVE_EMPTY=1` is an experimental, default-off daemon path.
It avoids the daemon's whole-tree source stamp only when **both** active and
finished build maps are empty globally. It does not avoid running Cargo.
The normal child executes Cargo's complete freshness checks and build/test
behavior, including build scripts, proc macros, diagnostics and exit status.

## Safety boundary

- Eligibility and child insertion happen under the same daemon state lock used
  by ordinary requests and the watcher. Another request or a prebuild cannot
  appear between observing the empty maps and reserving the request key.
- `Build.stamp` is `None`, not a synthetic source hash. Such a build never
  matches a stamped request and never enters the finished/replay map, including
  after early output. No source equivalence is asserted.
- A stamped request racing the native execution rechecks the key under the
  lock before any reuse or insertion. An unstamped owner causes a fail-open
  refusal, so the client runs independently instead of overwriting the active
  reservation. Its Cargo invocation performs freshness checks normally.
- The watcher sees the occupied active key and retries after it finishes. It
  cannot turn the native execution into a prebuild or reuse its output.
- Existing request remembrance, watch registration, streaming, disconnect
  handling, process-group cancellation, and pointer-checked reaping are shared
  with the ordinary path. An unclaimed or early-served competing prebuild
  prevents entry, preserving the existing stamped cancellation path.
- No active or finished candidate means no cancellation/settling operation has
  work to do at the instant of reservation. Recent watch intents alone do not
  authorize reuse and do not prevent the native execution.

This is deliberately more conservative than checking only the same request
key. A finished candidate for another request still forces ordinary handling.
It is not a replacement for `JUSTRUST_BUILDD=0`: the daemon continues managing
watch intents, child lifetime, streaming and later candidate reuse. It simply
lets Cargo validate the request when there is nothing for the daemon to reuse.

## Regression coverage

Unit tests cover default-off and exact opt-in behavior, explicit prebuild
exclusion, global active/finished candidate exclusion, the active-key guard,
non-replayability even with early/prebuild flags, and pointer-safe reaping.
Existing daemon stream/early-output and source-stamp tests remain intact.
The actual `run()` dispatch is exercised for both agent and explicit prebuild
requests racing an unstamped owner: both must send `F` without changing the
active pointer or incrementing reuse counters.

Hosted correctness gates passed:

- `justrust check`: `20261029-224423427-3561944`.
- `justrust test --lib buildd::tests`: `20261029-224739534-3583896`,
  13 passed, 2 ignored.
- `justrust clippy -- -D warnings`: `20261029-224747316-3583840`.
- Uninstalled `justrust build --release`: `20261029-224502624-3566464`.

Measurement results and acceptance checks will be recorded below. This change
does not install a binary or promote a default. General session-weighted speed
claims require the comprehensive before/after suite and are not made here.
