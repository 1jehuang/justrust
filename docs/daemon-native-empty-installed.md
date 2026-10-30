# Installed candidate-free daemon acceptance, 2026-10-30

This verifies the default-off `JUSTRUST_BUILDD_NATIVE_EMPTY=1` implementation
in `75e009bb0`. See `daemon-native-empty.md` for its atomic reservation,
non-replayability, cancellation, and controlled same-binary comparisons.
Native Cargo always runs. This is not whole-command result replay.

## Installed identity and correctness

`cargo install --path . --locked` completed. `src/buildd.rs` SHA-256 was
`778dfa516e679009a5f62dc682fc9635256a17d49acbc0ba6153af71f2033cf6`
before and after installation. Installed executable SHA-256:
`238d84fe75b5794344d64329f89cde726c8dff7401eda67dcc0ada737845f280`.

Final-source hosted gates passed:

- Check: `20261029-225440775-3674003`.
- Full tests: `20261029-225443553-3674420`, 843 passed, 12 ignored.
- Strict all-target Clippy: `20261029-225449756-3674002`.
- Installed explicitly forced hosted strict all-target Clippy:
  `20261029-225704685-3694010`, 62 fresh units, zero compilation. Its 10.1 s
  process included 9.448 s synchronization/reconnection, not a warm latency
  sample. No authentication, admission, or billing path was bypassed.

## Shared daemon correctly declines the shortcut

The installed ordinary invocation `20261029-225555808-3684164` ran locally
because of the existing ten-minute recent-remote-failure backoff, not an
intentional default routing change. It rebuilt edited sources and passed.
The explicit hosted invocation above then succeeded.

Three installed opt-in local requests used the normal shared daemon while an
unrelated project's active/finished state existed. They correctly retained
source validation rather than taking the candidate-free shortcut:

| Run | Process wall | Source validation |
| --- | ---: | ---: |
| `20261029-225603646-3685839` | 813.412 ms | 590.8 ms |
| `20261029-225604523-3686018` | 896.113 ms | 677.6 ms |
| `20261029-225605236-3686127` | 699.186 ms | 496.9 ms |

All had 62 fresh units and zero compilation. These are evidence of conservative
fallback, not claimed shortcut hits or a controlled performance comparison.
No shared daemon was stopped to manufacture an empty state.

## Isolated installed shortcut acceptance

A new scratch `JUSTRUST_HOME` isolated only the test daemon. Controls were
`JUSTRUST_REMOTE=off`, `JUSTRUST_BUILDD=1`, `JUSTRUST_BUILDD_PREBUILD=0`,
`JUSTRUST_BUILDD_NATIVE_EMPTY=1`, and explicit
`JUSTRUST_TOOLCHAIN=nightly-2026-10-08`. This local mode is a controlled
acceptance experiment, not a change to default hosted selection.

The first invocation `20261029-225813101-3710489` took 57.554 s including a
one-time isolated toolchain installation and toolchain-change rebuild. It is
excluded from warm observations, not hidden as a cache hit.

Subsequent installed strict Clippy invocations passed:

- `20261029-225910643-3719519`: 164.025 ms full process wall.
- `20261029-225910808-3719614`: 136.757 ms full process wall.

Both reported 62 fresh units, zero compilation. The isolated daemon log records
`stamp native`, 0.0 ms source validation, and 0.7/0.8 ms lookup respectively.
Each request launched Cargo normally and was served by exactly one client.
This establishes that the installed shortcut executes, not that a result was
replayed. No comprehensive session-weighted speedup or default promotion is
claimed. The synthetic benchmark gate remains incomplete, and unrelated global
candidates intentionally prevent this experimental shortcut.

The isolated run summaries were exported from its own index before cleanup.
Its daemon was stopped and only its newly downloaded 1.6 GB toolchain and
127 MB cache artifacts were removed. Raw run records and daemon logs are
retained. Normal local `jr-ac20de5e7921` Clippy subsequently passed with
62 fresh units and zero compilation in `20261029-230013752-3727530`.
