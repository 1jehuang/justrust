# Installed native execution without a reuse candidate

Date: 2026-10-30 UTC. Implementation: `8705f9420`.

`JUSTRUST_BUILDD_NATIVE_NO_CANDIDATE=1` is a separate, default-off operational
fast path. It does not assert source-tree isolation and does not replay a
Cargo verdict. The original globally-empty `JUSTRUST_BUILDD_NATIVE_EMPTY`
contract remains unchanged. See `daemon-native-no-candidate.md` for the exact
locked same-key and same-session cancellation guards.

## Installation and final correctness

The first install was interrupted by a server reload. Resuming
`cargo install --path . --locked` succeeded. Frozen `src/buildd.rs` SHA-256
before and after installation:
`88df8cad11b88de0df09df44eaa42c8f8f8d0c0c2b3ca10c953b743eae2538b8`.
Installed executable SHA-256:
`f5bf020d183d3081d2420492f6e76fd823793faa1fb65cd7ce57cee52c3929fa`.

Final ordinary hosted gates all passed:

- Check: `20261029-231708182-3885114`.
- Full tests: `20261029-231720182-3886462`, 847 passed, 12 ignored.
- Strict all-target Clippy: `20261029-231725708-3885112`.

The final Clippy gate rebuilt two units because `CLIPPY_ARGS` changed. Its
9-second hosted wall is not a warm unchanged-command observation. The check
also incurred an 11.353-second reconnect/sync. Neither is hidden as a cache hit.

## Actual concurrent native Cargo acceptance

Raw evidence and harness are in `bench/results/native-no-candidate-20261030/`.
Two independent scratch Cargo projects share one isolated test daemon. One
contains a real `build.rs` that writes its PID and sleeps. While that script
is alive, a different-session request checks the second project's actual
library. Both requests set the new flag to 1 and the old flag to 0.

The installed daemon logged two `stamp native` executions, each with 0.0 ms
source validation. The probe returned exit 0, `attached: false`,
`replayed: false`, and one sharer. The daemon still reported the independent
owner running, with two served requests and zero coalescing. Closing only
the owner's requesting socket then terminated its real build script.
Neither project's output was reused by the other. The test daemon was stopped,
not the shared user daemon.

The saved previous installed binary ran the same harness successfully with
zero native stamps, establishing ordinary execution/cancellation behavior.
Baseline and candidate probe times are retained only as raw observations:
the baseline compiled the probe, whereas the candidate found its artifact
fresh. Their wall times therefore are **not** a speed comparison.

## Real shared-daemon workspace observation

The installed binary also ran local `justrust clippy --lib -- -D warnings`
in the normal checkout with the new flag and speculative prebuilds disabled.
Local mode isolates this local-daemon behavior, rather than changing hosted
defaults. The normal shared daemon was not stopped to obtain these samples.

The first invocation rebuilt the actual implementation edit
(`20261029-231724693-3888009`, 5.648 s full process wall) and is excluded from
warm observations. Subsequent runs each kept 62 units fresh and compiled zero:

| Run | Full process wall | Source validation | Lookup |
| --- | ---: | ---: | ---: |
| `20261029-231730330-3890942` | 281 ms | 0.0 ms | 9.5 ms |
| `20261029-231730605-3891080` | 258 ms | 0.0 ms | 4.5 ms |

Both shared-daemon log entries say `stamp native`. These observations establish
that the installed path works outside an isolated daemon. They are not a
matched comprehensive-suite speedup, an uncontended latency guarantee, or a
reason to promote a default. Machine-wide compiler/memory contention remained.

## Actual edited failure and byte restoration

After the successful socket probe, an owned scratch library received
`compile_error!("native_candidate_real_edit_probe")`. Installed native execution
returned exit 101 and that exact diagnostic in
`20261029-231844899-3903294`. Restoring the byte backup was verified with `cmp`;
`20261029-231845124-3903387` then compiled the restored library and returned 0.
The trap restored the fixture and stopped only its isolated daemon even on
failure. No production checkout source was edited for this acceptance.

Same-key active/finished entries and same-session wrong guesses still force
ordinary handling. Whole-command result replay remains unimplemented, and
the comprehensive matched synthetic gate remains incomplete. Both native
fast-path flags remain default-off.
