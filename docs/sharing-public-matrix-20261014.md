# Fresh public sharing matrix, 2026-10-14

## Observed result

The independent trusted fleet completed publication at **04:35:59 UTC**. Its final
protected log reports **9,193 new bundles (5.62 GB), 9,193 manifest lines, zero
build-script runs and 710 skipped**. The public index grew from 10,724 to 19,917
entries. The instance was observed terminated at 04:38:55 UTC, before its hard
three-hour shutdown. This is actual publication, not launch success.

Producer client source `fe472c6`, SHA-256
`04662c0de99f0da0d7b18c49efcbcbf9ff74a37ee169a8e4d887bcdaf4cf56fe`,
used `nightly-2026-10-08` and `jr-b7907f1fe8f7`, each with default and debug=0
profiles, with public reads explicitly disabled. The final index version is
`zmxT_4Qmghz19YzdAkWJXr7.evph3112`. Protected log and before/after index
hashes are preserved in `sharing-public-matrix-20261014.json`.

## Actual empty-cache consumption

A separate frozen client from exact source `ec42e0f2e92eb429ac5a020419f03e9a1288a15c`,
SHA-256 `2153a809b3f91826f4baa9acd81ad5ccc4fb7102f17aacb029469853f38b5a53`,
used independently empty Cargo, target and dependency-cache directories for each
compiler/profile cell. Its dependency-cache source is byte-identical to the
producer's. Private/account/peer remote caches and shadow mode were disabled.
Public fetch was forced with `JUSTRUST_PREBUILT_ALWAYS=1` for correctness, not a
claim about the automatic download cost policy. Local execution was approved
while peers held the account host, and made no account-host or global-install
changes.

The matched public fixture pins **memchr 2.8.3**, the version actually present in
the producer's public workspace locks. The earlier 2.7.6 safe-miss fixture and
records remain unchanged. Every command below exited zero with **one public
prebuilt hit and zero registry misses**. Every test command ran two runtime tests
successfully.

| Compiler | Profile | Check run | Test run | Matching keys in old index |
|---|---|---|---|---|
| nightly-2026-10-08 | default | `20261013-214103061-1444770` | `20261013-214104300-1444974` | Both present |
| nightly-2026-10-08 | debug=0 | `20261013-214456522-1484966` | `20261013-214457788-1485566` | Both present |
| JR full b7907f1fe8f7b788dbace9ca649ce4729408f943 | default | `20261013-214458922-1486135` | `20261013-214500157-1486737` | Both absent |
| JR full b7907f1fe8f7b788dbace9ca649ce4729408f943 | debug=0 | `20261013-214501534-1487602` | `20261013-214503018-1488205` | Both absent |

All eight exact base keys appear in the final index. The four JR keys were absent
before publication and present afterward, so these runs demonstrate fresh reuse
of this producer's new bundles, not merely legacy public hits. Check and linked
test dependency artifacts have distinct keys. The corrected full JR package
contains the same compiler commit as the producer's short ID, plus repaired
helper runtime closure. Actual restoration and runtime tests establish this
bounded compatibility, not general package interchangeability.

Exact accepted inputs are committed in `bench/fixtures/sharing-consumer-matched/`.
`Cargo.debug0.toml` is the exact debug=0 cell manifest, intended to replace
`Cargo.toml` in an isolated copied fixture. The path-free JSON receipt contains
per-cell fixture hashes, actual argv, exits, public hits, tests, base/output keys,
index membership and raw unit/key-input record hashes. All eight IDs are included
in `data/runs/sharing-20261014/2026-10.jsonl`, now 51 selected records. Slim export
redaction drops `--locked`; actual argv in the receipt comes from original
metadata, not reconstruction from slim rows.

## Retained harness failure and rollout boundaries

The six later compiler/profile commands all passed, but their outer shell then
exited 1 because a final guard looked for nonexistent `installed.json`. The
non-mutating correction read the actual `justrust-toolchain.json` and verified
`installed_at=0`. Unpinned status remained system. The corrected full JR CDN
install took 38.1 seconds on this local runtime and remained inactive. This
harness failure is retained, not relabeled as a passing shell or compiler error.

Future fleet template contract `7e66628` adds a default-off opaque status receipt
flag and accepts the full immutable JR ID. Fourteen scoped contracts, the full
174 Node + 11 funnel/early-launch + 17 dashboard tests, and AWS template validation
passed. These are source/schema checks, not an observed future receipt or new
tenant bootstrap. Live safe-client pin advancement is held for the newly found
host-native CPU/opaque response-file cache eligibility guard.

No speedup is claimed. Forced fetching, existing nightly index entries, concurrent
work and ordinary functional timings are not a controlled before/after benchmark.
Genuinely new-tenant bootstrap remains unobserved. Portable native build-script
closure remains unauthorized, and registry archive seeding stays default-off
because full-network measurement was slower than direct Cargo fetching.
