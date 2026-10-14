# Fresh public consumer observation, 2026-10-14

## Result: safe miss, not public reuse acceptance

An independently empty Cargo home, target and dependency cache used the accepted
immutable nightly bundle and exact client from `ec42e0f2e92eb429ac5a020419f03e9a1288a15c`.
Client SHA-256 was checked before and after both commands:
`2153a809b3f91826f4baa9acd81ad5ccc4fb7102f17aacb029469853f38b5a53`.
The compiler pin was `nightly-2026-10-08`, rustc commit
`1d81eb4ad9cd207e3e638bd32b17ec4fce8412a6`; recorded Cargo was the bundle's
static `jr-cargo`, not the experimental scheduler candidate.

These local commands were approved as a distinct-runtime empty-cache integration
check while another agent held the hosted account. They did not install a global
client, modify the account host, write to the public store, or bypass billing.
Private/account/peer remote caches and shadow mode were disabled. Public fetching
was explicitly enabled with `JUSTRUST_PREBUILT_ALWAYS=1` for this functional probe.
This overrides the download cost policy only; it does not override key, manifest,
content, ABI or source validation. No speed claim follows from these runs.

| Workflow | Exact run ID | Exit | Public hits | Registry misses | Observed result |
|---|---|---:|---:|---:|---|
| `justrust check --local` | `20261013-212740419-1382686` | 0 | 0 | 1 | memchr compiled locally |
| `justrust test --local --locked` | `20261013-212741514-1383002` | 0 | 0 | 1 | memchr compiled locally, two tests passed |

The fixture pins public `memchr = "=2.7.6"`, lock checksum
`f52b00d39961fc5b2736ea853c9cc86238e165017a493d1d5c8eac6bdc4cc273`.
The check emitted metadata; the test dependency emitted metadata and link output.
They therefore have different exact base keys:

| Mode | Base key | Output key | Cached index membership | Direct CDN manifest on 04:32 UTC |
|---|---|---|---|---|
| check | `4c1f7a8ef8a6446fc1605b03190ca832` | `238081e2df8dd8ad6402d6312190afb5` | absent | HTTP 403 |
| test dependency | `710d9e80a92db201748afe134887f01a` | `a3bd42a1cf8cff3317c313b969514112` | absent | HTTP 403 |

S3 serves absent objects as 403 when public listing is disabled. Together with the
index absence, this supports an unpublished-key diagnosis, not a corrupted restore
or a successful public hit. It does not establish which producer determinant
(version, unified features, compiler, profile, arguments or environment) differs.
The consumer entries were saved only to its private local cache. Its compiled
artifacts were not promoted to trusted public output.

Both run summaries are in `data/runs/sharing-20261014/2026-10.jsonl`. The exporter
redacts this scratch repository and crate names. The reviewed fixture sources are
under `bench/fixtures/sharing-consumer/` so the public inputs are still reproducible.
The metadata retains the exact original argv and local route in the private run
record. The slim export drops `--locked`, so the table does not infer argv from the
slim row alone. Recorded background load was nonzero; these are correctness
observations only, not uncontended timings.

## Independent producer remains pending

At 04:33:07-04:33:09 UTC, the separate fleet instance
`i-0e641b79dc37fb3a3` was still running (`c7i.8xlarge`, launched 03:21:05 UTC).
The index remained version `uZL8OaIK3KwbCIsu.SX2Sg9S74P1Sjkd`, modified
02:52:23 UTC, 353892 bytes. The latest completed log was still the earlier
`fleet/logs/20261014T025222Z.log`. This is not completion or rejection of the new
compiler/profile matrix. No new matching publication or fresh public hit has yet
been observed. Keep the running producer and its hard lifetime unchanged.

## Future-only stage receipt improvement

Cloud commit `232b53a` adds an opt-in sanitized stage receipt to `prebuild/run.sh`.
The running producer downloaded its script before this commit, and no runner
upload, template update or receipt activation was performed for this improvement.
Four new receipt tests plus eight existing executable runner tests pass. Receipts
are whitelisted and contain no source, private paths, environment or credentials.
They require an explicit status ID and a selected verified client digest, preserve
build exit/upload ordering if observation fails, and report cumulative optional
failures as `partial`, never as complete success.

Stubbed runs observed 16 status writes and 16 date calls for one compiler, 23 each
for two compilers, with no per-build-command growth. This is bounded bookkeeping,
not a measured production overhead estimate. The then-current full cloud Node
suite passed 156 of 157 tests; its sole unrelated native-runner command-budget test
failed. Because npm short-circuited before Python, the 11 funnel/early-launch and
17 dashboard tests were separately run and passed. The native owner was notified.
No full-cloud-suite or live-status success is claimed.
