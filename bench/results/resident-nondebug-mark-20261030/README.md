# Rejected native nondebug-mark memoization evidence

**REJECTED. No promotion or speedup claim.** Licensed Blaze-derived resident,
not clean-room. Four strict native direct-oracle/provider-entry fixtures passed,
as did all 50 codegen and 359 resident tests. The matched frozen-v28 versus
v29b scoped timing did not improve: engine median **86.112926 ms to 91.814848
ms**, raw **+6.62%**, reference-normalized **+19.41%**. Noise is unknown. This
is descriptive, not significance or a causal slowdown claim, not the
comprehensive session-weighted workload, and not a default-promotion gate.

## Portable offline replay

With Python 3.11+ and only the standard library, from any working directory:

```sh
python3 /path/to/resident-nondebug-mark-20261030/validate.py
JCODE_SCRATCH_DIR=/writable/scratch python3 /path/to/resident-nondebug-mark-20261030/test_validate.py
```

The validator never runs a native tool, compiler, build, test binary, network
client, git, or copied root command. Historical absolute paths are receipt
values only and are never dereferenced. It needs only this copied directory,
not the checkout, sibling evidence bundles, original scratch, or installed
binaries. The test suite runs only offline mutations and one copied-validator
subprocess with Python isolated mode, an empty PATH, a fresh HOME and an
unrelated working directory. It does not run a compiler/runtime workload.

Offline controls are run only after root releases the protected native window.
Their actual result is retained in `offline-tests.log`; `audit.json` retains
the actual positive replay report. Neither is a new native test result.

## Raw contents and provenance

- `source-ready/v29/`: original full bytes, exact unbuilt explicit-LRU proposal,
  documentation, unified patch, SHA-256 inventory, and original source-only
  readiness. The LRU policy was rejected before root built it.
- `source-ready/v29b/`: exact first no-LRU/no-fixture proposal and honest
  source-only readiness, preserved without overwriting it.
- `source-ready/v29b/native-fixtures/`: full final candidate, original patch and
  Cargo manifest bytes, exact candidate manifest, pre-fixture source/doc
  snapshots, documentation, diffs, hashes, and final source-only readiness.
  That readiness predates actual native execution and correctly says proof was
  not yet established. Later passing raw journals do not rewrite that history.
- `native/build/`: actual root check/build logs/exits, raw justrust runs,
  engine identity/version, all 20 selected frozen full-source files, selected
  source hashes, and the 3,757-file all-resident identity inventory. Matching
  post-build and post-test receipts prove those identities stayed unchanged.
  The 3,757 source files themselves are not all duplicated here.
- `native/unit/`: all original 4-fixture, 50-codegen, and 359-resident logs,
  exits, and raw `meta.json`, `summary.json`, and `output.jsonl` journals.
  Individual passing test names and counts are checked against immutable
  recorded sets, not inferred from an empty filter or filtered wrapper output.
- `sessions/{rsb20,rsa26}/`: every original top-level file, byte-for-byte,
  including all 22 records per session, all stdout/stderr, version outputs,
  source original, suite/metadata/records/summary. All five edit samples per
  stack are retained with no exclusions. Engine/reference version, prepare,
  type-error, type-recovery, assertion-failure, assertion-recovery, and edit
  gates are checked against the raw outputs and the original test contract.
  Daemon homes, store/cache directories, binaries and build output are omitted.
- `provenance/`: original root build/test script, unchanged matched comparison
  output, frozen-v28 engine identity, exact original/rejected lock and unified
  lock patch. The candidate lock differs only by three existing devdep names
  in the codegen package. All 592 package records and versions are otherwise
  identical. Scripts retain historical paths but are not replay instructions.
- `harness-captured/`: suite/harness source captured during archival, not a
  claim that every earlier native session used this later capture verbatim.
- `licenses/`: resident license and source-attribution texts.
- `decision.json`, `restoration.json`: explicit rejection and observed exact
  two-path restoration receipt. Root separately restored the lock and later
  worked on an unrelated cold-diagnostic issue. Replay never reads live HEAD.

## Audit contract

`validate.py` checks the exact original/candidate source identities and replays
all three archived source patches, the manifest patch, and the lock patch
in memory. It validates final no-LRU policy, independent original direct oracle,
source-only readiness chronology, selected/all source freezes, actual named
native tests, all runtime success/error/recovery markers, edit-source parity,
profiles, tool/toolchain/binary identities, and all-sample medians. It recomputes
raw and reference-normalized differences and requires consistency with the
unaltered comparison and explicit REJECTED decision.

`manifest.json` is a complete SHA-256 inventory of retained files, including
validator, tests, collector, sealer and this README. Only the manifest itself
and derived `audit.json`/`offline-tests.log` are excluded to avoid recursion.
`seal.py` refuses to overwrite an existing manifest. A checksum inventory is
not a digital signature or a new attestation of native execution. The enclosing
git commit versions the audit implementation and immutable identity constants.

Semantic mutations rehash altered session artifacts, nested suite/metadata
copies and source-ready/outer inventories before requiring rejection for the
specific semantic reason. Controls cover source and ready drift, source patch
replay, native names/filter/status, source freezes, runtime/error/recovery
markers, source/toolchain/binary identity, profiles, medians, exclusions,
restoration, rejection policy, checksum corruption and detached relocation.
Original artifacts are never changed by those tests.

The inherited precise cold-matrix diagnostic ordering gap is separate from
this mark-query experiment. No v30 cold-support or performance result is
claimed in this bundle.
