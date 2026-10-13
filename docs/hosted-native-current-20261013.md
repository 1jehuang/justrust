# Current native admission recheck, 2026-10-13

## Decision

Do not restore the removed native HTTP library or change default transport.
Both `bench/suite.py compare` reports classify every workload within noise.
There is also possible shared-host contention in the middle epoch. This does
not establish the historical native regression is fixed or justify a new
whole-loop speed claim. 10x stock-local remains unmet.

## Exact experiment

Three native/curl/native epochs, each with two warmups and11 measured samples
for each workload, used the same frozen historical client `6c0988cb186b`:

```text
SHA256 e3376665dd4108ba21cb0b3c140f093d388616fc9c4727957dc4e4da080e8152
JUSTRUST_HOSTED_HTTP_POOL=1 / 0 / 1
JUSTRUST_REMOTE_TRACE=1
JUSTRUST_BUILDD=0
JUSTRUST_EARLY=0
JUSTRUST_EXIT_AFTER_EARLY=0
JUSTRUST_CARGO_DAEMON=0
JUSTRUST_TEST_FILTER_COMPILE=0
bench/suite.py run --remote --toolchain nightly-2026-10-08 -n11 --warmup2
  desktop-noop-test desktop-body-check desktop-test-edit
```

The pinned snapshots, own M8azn host, persistent remote target, and client SHA
were held constant. Actual admission transport was checked from existing
local-client stderr, not inferred from configuration. All99 measured samples
ran hosted with successful full verdicts. Every no-op compiled zero units;
every reference edit compiled one unit and passed three FPS tests. All six
bounded client timestamps were retained by the new benchmark parser.

| Workload | Native before median (range), s | Curl median (range), s | Native recheck median (range), s |
|---|---:|---:|---:|
| no-op test | .691 (.629-.893) | .778 (.710-1.068) | .709 (.628-.909) |
| body check | 1.855 (1.781-2.666) | 1.990 (1.819-2.391) | 1.839 (1.775-2.012) |
| reference edit | 5.185 (5.090-5.731) | 5.280 (5.141-5.516) | 5.221 (5.119-5.640) |

Native-before to curl: +13%, +7%, +2%, all within noise.
Curl to native-recheck: -9%, -8%, -1%, all within noise.

| Client charged milestone median, ms | Native before | Curl | Native recheck |
|---|---:|---:|---:|
| no-op test | 346.7 | 397.8 | 344.6 |
| body check | 334.5 | 446.8 | 353.0 |
| reference edit | 352.0 | 398.4 | 377.4 |

These are cumulative client timestamps, not isolated billing execution times.
The historical client does not emit the newer server admission phases, so none
are inferred. Compiler medians remained approximately1.06s for body check
and4.39s for the reference edit.

## Disclosed concurrency and cold starts

Another optimizer reported potential hosted load on the same account machine
from22:28:33 through22:30:58 UTC. This conservative window intersects all11
curl reference samples. The controls artifact explicitly identifies them using
actual run start plus client wall. Original sample `noisy` flags describe
local observations and were not rewritten to pretend they detect remote load.
The peer confirmed no backend/cloud deployment, D1, quota, binding, instance
type or region changes during this window. Its independent regional artifact
is not staged in this commit.

The suite correctly waited for the shared snapshot lock before the final
epoch. First native warmups had18.037s and17.057s synchronization costs after
daemon replacement/reconnect. They are preserved, not counted as measured
transport samples. No new billed request was made to repair a verifier.

## Default-hosted restoration and acceptance

The EXIT trap restored the current installed client and its default daemon.
Check `20261013-153259992-3622754` passed with zero compiled and56 fresh.
Its cold synchronization took10.462s, with about11.0s total client wall.
This is recovery correctness, not a warm speed comparison.

Subsequent plain commands, without a forced remote flag or frozen client:

- `justrust check`: `20261013-153339304-3627414`, hosted,0compiled56fresh.
- `justrust test`: `20261013-153339994-3627461`, hosted,360passed0failed2ignored,0compiled56fresh.
- `justrust clippy --all-targets -- -D warnings`: `20261013-153342838-3627413`, hosted,0compiled56fresh, exit0.

All121 exact owned records are exported:99 measured,18 warmups, one recovery
and three plain acceptance commands. Raw results and sanitized controls are
`bench/results/hosted-native-current-{native-before,curl,native-recheck,controls}-20261013.json`.
All24 harness tests pass. Peer findings, source changes and regional artifacts
are excluded. The host window was released to the regional optimizer only
after final dogfood acceptance. No source or live change was made by this
experiment, and no tenfold or clean-full-Desktop-suite claim is made.
