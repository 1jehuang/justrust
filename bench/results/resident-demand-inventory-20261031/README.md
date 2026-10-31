# Native demand inventory, frozen evidence only

This bundle binds accepted source commit `a59351f652c9b266b7f29734f2218cbb19a9aaad`
to frozen source bytes and historical native receipts. It implements evidence
retention and offline audit only. The evidence worker did not read live toolchain
sources, recapture source identities, compile, run native workloads, or edit root
documentation. All permanent changes are inside this new directory.

## Precisely bounded result

`RA_JIT_DEMAND_INVENTORY=1` is a separate, exact, default-off opt-in. The native
`ObjectCodegen` snapshot is taken after a successful full round and before
`finish`. It is stage-1 native-module-input metadata, not exact final object or
library closure. Typed data definitions are recorded only after native definition
succeeds. Database-local function, instance, module, type and data IDs are not
stable cache keys. Unwind/backend finalization data is not fully covered.
Retained-base patch returns have `inventory: None`, meaning unavailable coverage,
not zero work. No reusable persistent artifact, cache/read certificate, narrowed
exports or merged patch/base closure is enabled or certified.

The cold off/on diagnostic records both startup controls explicitly:

| call | RA_JIT_DEMAND_STD | RA_JIT_DEMAND_INVENTORY | result |
| --- | --- | --- | --- |
| off | 1 | 0 | four actual summary tests, native link, no inventory report |
| on | 1 | 1 | four actual summary tests, native link, full-round inventory |

Enabled raw worker output reports 6,998 compiled roots, 0 skipped roots, 48,841
declared instances, **0 found-instance map entries**, 0 rejected instances,
2 kept statics and 26,533 successful data definitions. The empty live found map
must not be interpreted as a full history of dependencies. Both calls retain
source, tool and 4,020-file toolchain identities before/after unchanged. These
two cold calls do not prove semantic equivalence for every workload or zero
overhead.

## Frozen provenance, including the failed candidate

- `frozen/v34-history/` preserves the earlier barrier report and original
  before/after snapshots for history. It is not native acceptance.
- `frozen/v35/` preserves the worker originals, patch, SHA256SUMS and historical
  `FROZEN_READY` statement verbatim. That statement was source-only, not compiler
  readiness. `root-freeze/` contains root's separate bounded three-path freeze.
  Its bytes match the worker snapshots. Root took ownership after stopping the
  source worker. This evidence worker did not witness or repeat that stop and
  does not upgrade the worker's historical readiness claim.
- `candidate-build-v35` actually failed with three fixture compiler errors:
  one borrowed `Arc` mismatch and two unsupported relocation-target equality
  operations. The complete raw journal, displayed error output and exit 101 are
  retained. This failure is not relabeled as acceptance.
- `frozen/v36/` is the distinct root-owned fixture repair. The validator checks
  that only the data fixture changed: clone the target `Arc`, then assert exact
  `ModuleRelocTarget::User` namespace/index for both captured and original data
  and function relocations. The main source is byte-identical to v35.
- `accepted/` comes from `git show` of the accepted commit, not a live checkout.
  The saved gitshow patch is replayed against the frozen original source, and
  both accepted Rust files bind byte-for-byte to v36. The accepted document adds
  root's actual results to the source-only v35 document. It is not substituted
  for that older document.

## Actual native gates, retained rather than re-executed

| gate | justrust run | actual result |
| --- | --- | --- |
| v35 check | 20261030-182508083-573757 | exit 101, three compiler errors |
| v36 check | 20261030-182658166-583351 | exit 0 |
| v36 build | 20261030-182713472-585395 | exit 0 |
| v36 codegen tests | 20261030-182838650-595329 | all 48 passed |
| v36 resident tests | 20261030-182858112-597752 | all 360 passed |

`journals/` retains each **entire** original `output.jsonl`, `meta.json` and
`summary.json`, including all warnings and test names. `gates/` retains displayed
logs, exits, 3,757 source identities, the all-source after-check and lockfile
before/after receipts. No target, executable, private cache, or telemetry home is
copied. `slim/` is a derived path-free receipt, not a replacement for raw journals.
The new two codegen tests and one resident identity test occur in raw native
output. Build identity is
`resident-v1-11eee3f9ddd82894df5699baa288843febacff7987f3ab821b193720335599e5`.
The engine SHA256 is
`767491d3f56ba67ae4c87d7275442cb9b6fae62cfa06f4538c5e3c565dbda301`.

`runtime/rsb21` and `runtime/rsa27` preserve both entire 22-row sessions:
versions, cold preparation, type error, assertion failure, both recoveries, and
five executed edit markers per stack. Source edits are reconstructed with the
frozen helper and compared to every recorded source digest. The source-built
reference, before engine and after engine identities remain distinct. Every
session retains full raw stdout/stderr, source original, metadata, records,
summary and suite hashes. Safe historical environment maps omit some startup
flags. Those maps remain unchanged: absence is not an explicit flag value.
The separate probe's `startup_controls` are the authority for its explicit
STD=1 and INVENTORY=0/1 calls. Runtime suite flag omission is not filled from
ambient environment or inferred from successful output.

Engine edit medians are 0.08207886498712469 s before and 0.0798251059895847 s after.
Reference-normalized change is -4.916878153637749%, below the 10% diagnostic floor.
Noise is **unknown/uncontrolled**. This is descriptive scoped regression evidence,
not a speed win, a comprehensive workload gate, default promotion, or proof of
zero semantic/performance overhead. `outer/32891831af.output` preserves root's
full 733-test Python log, including expected CLI-error fixtures. The separate
`306723vuyn.output` is only the outer runtime completion receipt.

## Portable audit

Run from any directory with Python 3.11+:

```sh
python3 -B /path/to/bundle/validate.py
python3 -B /path/to/bundle/test_validate.py
```

`SHA256SUMS` inventories all retained files except itself and the reproducible
`validation.json` report. No Git, checkout, original HOME, engine binary, cache,
network, compiler or toolchain installation is needed. The validator imports
only independently SHA-pinned frozen Python helpers and calls their pure edit
and output-parsing functions. It never invokes their workload entrypoints.

`audits/offline-25-controls.output` retains the successful full replay and all
25 offline controls. `audits/offline-historical-prefix-attempt.output` also
retains an earlier audit-implementation failure: v34 checksum paths carried an
original directory prefix. The validator now maps that prefix explicitly while
preserving the original receipt bytes. This was not a new native failure.

The tests include positive replay with subprocess creation forbidden, plus
relocation into a no-checkout temporary copy with empty HOME and unusable PATH.
An audit hook forbids subprocess/network operations and opening any original
`/home` path outside that copy. Negative controls coherently resign the outer
inventory and the nested runtime suite artifact hashes before semantic rejection.
They cover exact startup flags, positive inventory counts and found-map history,
full-round/off absence, native four-test output, tool/TC/source identities,
preserved compiler failure, raw full test names, the fixture repair, accepted
patch replay, executed markers, type errors, recovery, row coverage, environment
allowlists, unknown noise and the complete Python root receipt.

`collect.py` is the original one-shot retention recipe, not a replay dependency.
Do not rerun it to recapture from a changed checkout. `--seal` is only for creating
or explicitly testing a new evidence inventory, never proof of authenticity.
The immutable source/helper/tool/TC commitments and semantic joins are separate
from the resignable transport hashes. Root owns independent final replay.

## Licenses and attribution

The frozen compiler sources are from the licensed Blaze-derived native resident
line, not the clean-room compiler. Original source attribution and the previously
retained `LICENSE-MIT` and `LICENSE-APACHE` are in `licenses/`. `input/source63/`
preserves all 63 pinned source inputs including their LICENSE, NOTICE and server
license. Frozen Python helpers retain their original headers and provenance.
No binaries, targets or private caches are included.
