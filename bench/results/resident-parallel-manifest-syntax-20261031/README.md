# v81 parallel manifest syntax: rejected evidence archive

**Decision: REJECT.** OFF median 0.05775033299869392 s, ON median
0.052218245487893 s, observed reduction 9.579317076710248%. The original
comparison reports -9.58%, below the strictly greater than 10% promotion floor.
Noise is unknown. No significance, causal, general-speed or default-promotion
claim is made. This archive is only v81, not the later v82 experiment.

## Portable offline commands

From any copy of this directory, using Python 3.11 or newer:

```sh
python3 -B validate.py --manifest
python3 -B test_offline.py
```

The tests relocate the whole bundle and launch the same Python interpreter with
`-I -S -B`, empty `HOME` and `PATH`, and an audit hook denying external reads,
external directory listing, writes, subprocess/native launches, sockets and
ctypes. Six real denial attempts are asserted. Python and its stdlib are loaded
before installing the hook. This is a bounded audit harness, not an operating
system sandbox or a claim that arbitrary Python is safe. No compiler, Cargo,
engine, network connection or private data is needed by portable validation.

`validate.py` runs the exact selected original strict-oracle functions from
hash-pinned source before checking historical commitments. It does not execute
archived orchestration scripts. Semantic negatives reseal `SHA256.json`, so they
exercise checks independent of that outer seal. A separate coherent timing/result
rewrite is caught by fixed historical commitments. The commitment is an integrity
anchor in the validator source, not a signature or independent provenance proof.
`seal.py` regenerates artifact inventories only. It does not silently update the
validator's fixed commitments and must not be used to bless changed evidence.

## Explicit export scope

- `source/`: exact original two production files, candidate five files, complete
  patch, initial source-only RELEASE, static checks, licenses and attribution.
- `harness/`: original two files, candidate three files, patch, STOPEDITS,
  RELEASE and native-free harness test logs (18, 113 and combined 131 tests).
- `oracle/`: five original accepted-base Python sources. `drivers/`: historical
  orchestration source retained for inspection only.
- `gates/`: check/build and four native test logs, all full recorder output
  journals, compiler/version identity, lock and 3,759-file source verification.
  Native named tests: 28 manifest, 6 metadata, 53 codegen and 373 full-native.
- `inventories/toolchain.json`: the 4,020-entry identity shared by the original
  before/after inventories. Runtime source inventories contain 63 files.
- `profile/`: eight edit/noop raw stdout/stderr calls and selected complete
  trace events, with original trace hashes retained in state/input receipts.
  Trace files here are allowlisted projections, not byte-identical full traces.
- `strict/`: original fixture bytes and raw stdout/stderr/status/worker/daemon
  outputs for OFF, ON, ON-repeat, OFF-repeat. `observations.json` contains explicit
  field projections of all four states, totaling 28 phases.
- `abba/`: actual 54 calls in reference plus OFF/ON/ON/OFF order, 20 retained
  timed samples including all first touches, warmups, failing/recovery gates,
  raw status receipts, process PID/start-tick/exe/socket/full-argv controls,
  original fixture bytes, combined unchanged-byte manifest/lock retouches,
  original result projections, original comparison and stop receipts.
- `cleanup/`: post-measurement stop outputs for four strict workers, with no
  persistent store deletion. `private-inputs.json` records 474 explicitly read
  input roles, original byte lengths and SHA256, verified unchanged after export.

The author-only `export_private.py` accepts explicit input directories and exact
prefix-to-role redaction mappings. It exports no ambient environment object,
credentials or binary payload. Raw text stdout/stderr and complete native
recorder journal entries are preserved with mapped private absolute prefixes.
Absolute private locations become tokens such as `<native-root>` and `<abba>`.
The exporter asserts source/harness rollback live backups match frozen candidates
and shared before/after toolchain inventories are identical before projecting.
The portable validator cannot reread those private originals. Their original
hashes remain distinct from hashes of redacted public copies.

The native restoration happened before v82 ownership was released to another
worker: two production files restored from exact originals, two new test modules
removed after preserving all five live files. Historical `cmp` and HEAD diff
checks passed then. This does not assert the current native checkout is unchanged.

## Replay coverage and limits

Replay joins original/candidate unified patches, proves the unchanged common
semantic body and frozen original native oracle body, joins candidate Rust hashes
to the actual native source inventory, verifies all named native outcomes and
raw recorder shapes, reconstructs every strict source edit and original diagnostic
oracle, checks trace ordering/nesting and runtime markers, checks exact ABBA flags,
nonces, process identities and unchanged bytes, and recomputes all medians from
the retained samples. Original native execution was performed by the root, not
by this exporter or replay. Preserved positive results are not a universal
correctness proof. Instrumented profile calls do not establish a causal speedup.
The requested broader promotion floor failed, so the candidate was restored.

Original root gate task references: compiler/full crates `483440v3r0`, positive
activation eight calls `8495495tqp`, strict 28 phases `9041478cjt`, pinned ABBA
54 calls `212737vvc8`. These references identify historical execution, not new
jobs or a dependency for portable replay.
