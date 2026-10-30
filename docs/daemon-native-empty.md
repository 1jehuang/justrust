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
- Complete hosted `justrust test`: `20261029-225215761-3651818`,
  843 passed, 12 ignored.

## Controlled local evidence, 2026-10-30 UTC

Results: `bench/results/daemon-native-empty-20261030.json`. This is a focused
source-stamp diagnostic, **not** the comprehensive session-weighted suite.
There is no default promotion and no broad speedup claim.

The coordinator confirmed a source/build freeze before the final samples.
One copied release candidate was used for both arms, with the experimental
flag set to `0` or `1`. The current installed binary was saved separately
before development rather than reusing the older sort-experiment baseline.
The paired experiment intentionally uses the **same new binary**, avoiding a
version difference as a confounder.

Both arms used `JUSTRUST_REMOTE=off`, `JUSTRUST_BUILDD=1`,
`JUSTRUST_BUILDD_PREBUILD=0`, and the explicit installed
`JUSTRUST_TOOLCHAIN=nightly-2026-10-08`. This local exception is necessary to
measure the local daemon's validation cost rather than hosted transport.
An isolated `JUSTRUST_HOME` supplied the daemon and recording state, with
`toolchains` symlinked to the normal installed toolchains. Both arms reported
the same compiler pin and sticky target slot 2.

| Warm real-repository command | Off median | On median | Samples per arm |
| --- | ---: | ---: | ---: |
| `justrust check --lib` | 777.552 ms | 184.918 ms | 6 |
| `justrust clippy --lib` | 751.823 ms | 173.682 ms | 3 |

These are complete client-process wall times. Check ordering was
`[off,on,on,off]` repeated three times, Clippy ordering `[off,on]` three times.
Warmups are excluded. Every timed no-op reported zero compiled units and
62 fresh units, so both commands still executed native Cargo's freshness
path. The daemon recorded 524.1–653.1 ms source validation in the check off
arm and 540.2–595.0 ms in the Clippy off arm. Native executions recorded
0 ms source validation, with 1.1–2.0 ms and 1.2–1.3 ms lookup respectively.

Machine-wide quiet was not achieved: unrelated workloads remained active.
The pinned warmup reported 9.6 other CPU cores on average and up to 30 other
rustc processes. Absolute wall times and their variation are therefore
contention-dependent. The same-source, same-binary alternating observations
identify the avoided daemon scan, not an uncontended latency guarantee.
Cold, release, hosted-latency and comprehensive workload comparisons were
not run for performance promotion.

### Real edited failure and byte restoration

During the coordinated freeze, a single deliberate
`compile_error!("daemon_native_empty_controlled_edit");` was inserted in
owned `src/buildd.rs`. Both off and on invocations returned 101 and printed
the complete compiler error rather than replaying a previous successful
result:

```text
error: daemon_native_empty_controlled_edit
   --> src/buildd.rs:710:1
    |
710 | compile_error!("daemon_native_empty_controlled_edit");
    | ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

error: could not compile `justrust` (lib) due to 1 previous error
```

The file was restored from its byte backup, not Git, and `cmp` passed.
Both subsequent commands returned 0. The first restoration command compiled
one local unit, while the second found it fresh. Consequently their timings
are correctness evidence, **not** a matched restoration speed comparison.

| Phase | Flag | Exit | Wall | Run |
| --- | ---: | ---: | ---: | --- |
| Edited failure | 0 | 101 | 2849.718 ms | `20261029-225324718-3663930` |
| Edited failure | 1 | 101 | 2342.258 ms | `20261029-225327134-3664274` |
| Restored | 0 | 0 | 3233.442 ms | `20261029-225330299-3664853` |
| Restored | 1 | 0 | 141.329 ms | `20261029-225332986-3665248` |

Source SHA-256 before and after restoration:
`778dfa516e679009a5f62dc682fc9635256a17d49acbc0ba6153af71f2033cf6`.
Edited source SHA-256:
`f523a84425b5d8dc588150586c50ed44d4e86372fe6c601d958d795aebd98ec0`.
Paired candidate SHA-256:
`18b3dd42286c5df75d90fa238e1fcd409f27ac4941c24b02101d8e6bc2bed116`.
Saved installed baseline SHA-256:
`08fd9cc630cc44e4029ed92b7090d5994e007c805da97b48df10f42593042ebf`.

### Real cancellation and overlap acceptance

An independent scratch Cargo project had a `build.rs` that wrote its PID
and slept for 30 seconds. With the experimental path enabled, killing
**only the requesting CLI process** caused the daemon to log
`no clients left, stopping pid ...`. The actual build script exited within
454.478 ms of the client kill. The daemon had logged `stamp native` and
0 ms source validation. This exercises actual process-group cancellation,
not a mock or a substituted exit code.

A second acceptance used real Unix sockets with an actual sleeping build
script owned by a native execution. An overlapping identical agent request
and an explicit prebuild request each received exactly:

```text
F: native freshness build already owns this request
```

A daemon status request while the owner remained alive reported `running=1`,
`served=1`, `coalesced=0`. Neither request attached to the unvalidated owner
or replaced it. Closing the owner's socket then terminated its actual build
script. The retained `run()` regression additionally verifies the original
active `Arc` pointer is unchanged for both refusal cases.

These overlap tests cover explicit prebuild requests and active ownership.
They do not claim a new end-to-end watcher timing benchmark. Watcher retries
continue through the existing occupied-key branch, and the ordinary stamped
prebuild cancellation/settling code remains in use whenever any candidate
exists.

The worker did not install or promote the candidate. After all samples and
byte restoration completed, the coordinator took responsibility for the
normal installation workflow. Run records from the isolated measurement home
must be exported in addition to the normal hosted run records.
