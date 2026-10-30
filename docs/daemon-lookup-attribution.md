# Local daemon lookup attribution

The daemon's previous `waiting for a compile-only prebuild` duration started
before `source_roots` and `source_stamp`. It therefore included recursive
source validation, even when no prebuild was running. Historical logs showing
870–1,640 ms under that label cannot establish a prebuild-wait bottleneck.

Commit `bbf63226e` separates lookup time into source-validation and prebuild
coordination intervals. The latter includes stale-prebuild cancellation and
coordination, not just sleeping. Lookup also includes request identity and
build selection/start. No source filtering, stamp semantics, cache identity,
prebuild policy, or result reuse behavior changed.

Hosted correctness checks passed:

- Check: `20261029-220523313-3250217`.
- Eight daemon tests: `20261029-220530275-3251742`.
- Strict all-target Clippy: `20261029-220535109-3249540`.

The controlled local `JUSTRUST_BUILDD=0` experiment is a diagnostic only.
It still invokes native Cargo input validation and does not authorize cached
command-result replay. One attempted root comparison rebuilt `src/record.rs`
under severe concurrent compiler contention and is excluded from performance
claims.

## Installed observations

The combined recording and daemon changes were installed with
`cargo install --path . --locked`. Installed hosted strict Clippy passed
(`20261029-221012298-3290569`). A first local invocation rebuilt the recording
source and is excluded from warm measurements. Two subsequent unchanged local
invocations each validated 62 fresh units:

| Run | Full process wall | Daemon source validation | Prebuild coordination |
| --- | ---: | ---: | ---: |
| `20261029-221029310-3292520` | 1.213 s | 951.4 ms | 0.0 ms |
| `20261029-221030605-3292774` | 1.276 s | 1044.1 ms | 0.0 ms |

These observations establish a local recursive-validation bottleneck, not a
prebuild-wait bottleneck. They are not a matched before/after speedup comparison.
The diagnostic uses `JUSTRUST_REMOTE=off` to isolate local execution, without
changing the hosted default. All file traversal and Cargo validation remain.

## Focused pinned baseline attempt

A daemon-enabled local `bench/suite.py run desktop-noop-test -n 3 --warmup 1`
attempt used the existing installed binary and a 60-second diagnostic deadline.
It timed out during the initial Desktop test compilation, before producing a
result JSON. The daemon detected client exit and cancelled its owned child.
No before/after comparison or end-to-end speedup is claimed from this attempt.
The pinned Desktop snapshot's measured source validation was only 25.6 ms,
so it would not represent the 171,114-file local checkout's scan bottleneck.
The comprehensive synthetic gate remains incomplete as described in
`caching-levels.md`.

## Installed exact-order construction

Commit `03131115a` accelerates sorting without caching metadata or dropping
inputs. See `source-stamp-component-keys.md` for the exact-order/hash oracle
and optimized same-input component measurements. Final hosted check
`20261029-222118255-3389646`, 839-test suite
`20261029-222119357-3389792`, and strict all-target Clippy
`20261029-222124896-3389569` passed.

The final installation preserved source SHA-256
`43340eb24932ef28aa6ed840ffafc02484bc563b09fe12f9216b61a5033bfbb7`
for `src/buildd.rs` before and after the build. Installed executable SHA-256:
`08fd9cc630cc44e4029ed92b7090d5994e007c805da97b48df10f42593042ebf`.
Installed hosted strict Clippy passed (`20261029-222318036-3405439`), but
its 10.9-second reconnect sync is not treated as a warm timing sample.

After a discarded local source rebuild, three unchanged local invocations
again kept all 62 units fresh:

| Run | Full process wall | Source validation | Prebuild coordination |
| --- | ---: | ---: | ---: |
| `20261029-222335381-3407669` | 0.772 s | 572.3 ms | 0.0 ms |
| `20261029-222336240-3407789` | 0.861 s | 668.7 ms | 0.0 ms |
| `20261029-222337005-3407926` | 0.778 s | 573.4 ms | 0.0 ms |

These are installed observations under concurrent machine load, not a matched
pinned-suite comparison or a sustained/general subsecond guarantee.

A real reversible edit added an unused local in `src/runs.rs`. Strict Clippy
correctly returned exit 101 with that exact diagnostic
(`20261029-222439184-3415263`), rather than replaying the prior success.
Restoring from the backup was verified byte-for-byte, and strict Clippy then
passed (`20261029-222445435-3415911`). No temporary source change remains.
Whole-command result replay is still not enabled.
