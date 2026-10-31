# One cold resident profile, October 30, 2026

This is a **single profiled diagnostic**, not a paired optimization comparison,
a savings estimate, a comprehensive workload, or quiet-machine proof. It profiles
the experimental licensed Blaze-derived resident, not clean-room Rust.
The candidate source is committed `91220ff43`, and the frozen main source is
retained. It predates the separately running v31 standard-library demand
experiment. This evidence makes **no v31 validation or performance claim**.

## Actual raw observations

The actual runner made two cold native calls in distinct private stores, first
reference then candidate. Both direct stdout logs contain the exact same four
named summary tests passing, zero failed/ignored and 144 filtered tests. This
validates only that filtered test contract, not all supported language semantics.
The state receipt reports 63 input source files, toolchain and engine hashes
unchanged before/after. It retains unknown contamination and explicitly says
waited child CPU does not establish a quiet resident machine.

Candidate engine timing receipt:

| Reported interval | Value |
| --- | ---: |
| Total | 26,338.5 ms |
| Daemon | 22,983.0 ms |
| Client planning | 3,337.5 ms |
| Eager standard-library preparation | 9.1 s |
| Project cold preparation | 5.1 s |
| Source checking | 1.4 s |

The standard-library log reports 17 crates, 25,924 of 25,960 function roots
compiled and 16,200 extra instances. Project cold work reports 3,990 roots and
40,606 instances. Source checking reports 31 files. The log's CPU phase totals
include concurrent work: for example standard-library translate is 80.4 s.
These are **overlapping, not additive wall-clock intervals**. Do not sum them,
subtract one interval as assured savings, or turn the reference observation
into a performance comparison. Worker logs retain unsupported-operation details
and fallback output rather than hiding them.

## Contents and provenance

- `raw/`: exact stdout/stderr, complete state JSON and both actual worker logs.
- `runner/`: actual profile runner plus its three helper modules captured from
  the pinned source commit. Historical paths in these receipts are inert strings.
- `provenance/`: exact frozen candidate main, 3,757-entry compiler source hash
  inventory, source63 manifest, build exit receipts, binary identity/version and
  capture scope. This does not redistribute all compiler or source63 contents.
- `licenses/`: resident Apache/MIT licenses and imported source provenance.
- `validate.py`: standalone Python 3.11+ offline hash, native count/identity,
  actual four-test stdout and timing/phase replay. No native tools, checkout,
  network or historical absolute-path reads are required.
- `test_validate.py`: positive replay, corruption, coherently re-signed semantic
  mutations and relocated empty-PATH replay. Mutations update the inventory, so
  semantic checks must reject them beyond a checksum mismatch.
- `manifest.json` inventories all inputs including audit code. Only derived
  `audit.json`, `offline-tests.log` and the manifest itself are excluded.

Run from any directory with an absolute bundle path:

```sh
python3 /path/to/resident-cold-profile-20261030/validate.py
python3 /path/to/resident-cold-profile-20261030/test_validate.py
```

The collector is historical capture code, not part of portable replay. It reads
only frozen receipts and uses Git to retrieve committed helpers/licenses and
compare frozen main against that commit. It never launches a native workload.
The validator and controls never run the collector, original runner, or helpers.
SHA inventories identify retained bytes, not a cryptographic signature of the
host's execution. The raw state is the original run receipt, not a new run.
