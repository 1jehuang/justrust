# Published justrust stable versus ultrafast, November 18, 2026

## Result and scope

The published shipping client is now a separate benchmark arm. A fresh matched
five-repetition trial on the dedicated hosted machine passed every correctness
and identity gate for stock Cargo, the published client, and the owned
Blaze-derived engine. Each arm passed 36 workload checks, including the full
library oracle of 147 passed and one ignored test, with exact test-name/status
parity. No official Blaze executable was rerun in this trial. Its earlier
compatibility failures remain separately documented in
`docs/ultrafast-official-upstream-20261118.md` and are not ranked by speed.

**This measures the shipping client's explicit local mode on the dedicated
host, using the same pinned stock Rust toolchain as the Cargo arm. It does not
measure the default network-inclusive justrust hosted service, its shared fleet
caches, a separately installed jr-rustc/jr-cargo toolchain, or general resident
compiler performance.** The experimental engine is invoked directly, not through
`justrust native`. All Rust compilation and test execution happened on the
hosted machine. No developer-local native build or installation was performed.

## Five-sample medians

All values are seconds, including the client command's full wall time. Reading
stable's recorded test log is outside the timed interval. The final column is
stable divided by ultrafast for this fixture, not a session-weighted or universal
speed claim.

| Phase | Stock Cargo | Published stable | Owned ultrafast | Stable / ultrafast |
|---|---:|---:|---:|---:|
| Cold private stores | 9.174716 | 9.559100 | 20.571280 | 0.465x |
| Same-byte source rewrite | 0.928447 | 0.089821 | 0.035057 | 2.56x |
| Reachable test-body edit | 0.964211 | 1.002950 | 0.176903 | 5.67x |
| Type error | 0.581548 | 0.620266 | 0.081259 | 7.63x |
| Type recovery | 0.956764 | 0.999879 | 0.158528 | 6.31x |
| Assertion failure | 1.008378 | 1.057395 | 0.090403 | 11.70x |
| Assertion recovery | 1.011387 | 1.051033 | 0.090553 | 11.61x |

Stable is substantially faster than plain Cargo for the same-byte rewrite in
this run, but roughly comparable for a real body edit. Ultrafast is faster for
this fixture's warm changed-body/error/recovery commands, while its cold build
is more than twice as slow as stable. Neither a universal 10x claim nor a default
promotion follows. `warm-unchanged` is the historical phase name in receipts:
identical bytes are written again, so it is **not an untouched no-op build**.

## Exact inputs and execution

- Fixture: pristine public justrust `f97fcc2d7490ec3528a8d67657dc3f8d6d457a7d`.
- Published client: `justrust 0.1.0-alpha.37`, selected by the production release
  endpoint at 05:04 UTC. The published archive checksum was verified, and its
  extracted executable matched the staged host executable exactly. SHA256:
  `bd1be5b7b3c731ecc47760b611533c27d94e6b6ce3a3b6104c7be3858375fe66`.
  The executable's `update status` also confirmed its release-build marker.
- Owned engine SHA256:
  `f0f968042f3f9f0b8ff433361ea0812b6189f617975041a6ac8695bb00f356fa`.
- Existing complete `nightly-2026-10-27-x86_64-unknown-linux-gnu` toolchain,
  full before/after file manifest and hashes retained. Both stock and stable
  actually used that toolchain's Cargo and rustc, verified from stable's exact
  foreground run metadata for all 36 commands.
- Instance: `i-0e0843922bdb5f48d`, m7i.4xlarge, 16 logical CPUs, about 61 GiB RAM.
  All arms used its ext4 root filesystem in this trial. Boot identity observed
  during setup: `57388d11-597b-47be-b722-27608a7ceb7d`.
- Frozen harness commit `69d76edb1`, file SHA256
  `227fa84a4897c2d6106bbc059e13bf2aadbc2247418ec9d6c83f509d34c7bf6d`.
- Five fresh private target/client/engine namespaces, rotating first-arm order,
  matched source bytes within each phase, and fresh runtime nonces for changed
  successful/failing test execution. Cold means private stores, not flushed OS
  page caches. The host was dedicated, but process CPU snapshots do not certify
  a perfectly quiet machine.
- Stable command: `justrust test --local="controlled same-machine hosted comparison"`
  followed by the same Cargo/test arguments. Its private HOME, JUSTRUST_HOME,
  Cargo and XDG directories carry no operator account credentials. Updates and
  telemetry are disabled, and no experimental native-engine override is set.
  This preserves normal local client caching, reporting, and test-runner behavior.
- The native private prebuild daemon is stopped after each repetition and again
  before source restoration. These stops are outside measured test intervals.
  All fifteen source copies were restored, executable/toolchain/source identities
  remained unchanged, and cleanup reported no errors.

## Output-oracle correction, not a stable compiler failure

The initial 05:07 trial's adapter tried to parse the agent-facing console output.
Stable deliberately hides most passing test lines, so the exact-name oracle
correctly withheld medians. Exit status or a compact aggregate alone was not
substituted for the stricter contract. This was a benchmark integration issue,
not evidence that stable's compiler or tests failed. Its original records,
raw streams, and native recorded runs were preserved unchanged.

The corrected adapter keeps the ordinary `justrust test` invocation and its
wall time. After completion, it extracts the exact foreground run ID printed in
the report and reads `justrust log <exact-id>` outside timing. It never asks for
the latest run, which could instead select a background prebuild. Full recorded
output supplies the same exact name/status/count and fresh-marker checks as
other arms. The adapter verifies foreground metadata ID, source directory,
arguments, actual Cargo path, and actual rustc path, preserving the original
metadata and log hashes. Missing, ambiguous, failed, or mismatched log retrieval
fails acceptance. Forty pure Python mocked tests pass, including the full
four-arm orchestration contract, wrong compiler identity, unavailable logs, and
private daemon cleanup. The real corrected five-repetition capture then passed.

## Account-private original evidence

Qualified run IDs:

- `bench/ultrafast-published-stable-20261118-0516-metadata`
- `bench/ultrafast-published-stable-20261118-0516-summary`
- `bench/ultrafast-published-stable-20261118-0516-records-lossless`
- `bench/ultrafast-published-stable-20261118-0516-raw-lossless`
- `bench/ultrafast-published-stable-20261118-0516-native-records-lossless`

The lossless helper verified authenticated reconstruction of every original
byte in the records and raw bundles. The records array is 737,515 bytes, SHA256
`596a9e55e118f641105257a406d0766f08baef7e7672ea79fc824cd44cd8faa3`.
Raw bundles include original presentation streams, exact foreground recorded
logs and metadata, and stable's own native run records.

Initial adapter-failure evidence remains available as:

- `bench/ultrafast-published-stable-20261118-0507-records-lossless`
- `bench/ultrafast-published-stable-20261118-0507-raw-lossless`
- `bench/ultrafast-published-stable-20261118-0507-native-records-lossless`

Those archives likewise verified exact authenticated reconstruction. Their
receipts were not rewritten to make the initial adapter pass.

## Shutdown

After original evidence archival, the dedicated instance was stopped and its
`stopped` state verified at 05:23 UTC on November 18. Encrypted disk
`vol-0bc1075a06eec2b20` remains retained. No installed developer default, production
routing, account credentials, billing rules, or tenant data changed.
