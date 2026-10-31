# Balanced manifest syntax v82: rejected targeted experiment

**REJECTED.** Same-candidate OFF median 0.05177067649492528s versus ON
0.052115632497589104s, **0.666315% slower**, rounded +0.67%.
The greater-than-10% reduction floor was not reached. Noise remains unknown.
This is only warmed noop/clientplanning combined same-byte manifest+lock
retouch, not separate-file attribution, significance, default promotion,
general agent speed, or session-weighted benefit. Historical v81 remains a
separate below-floor rejected experiment.

## Standalone replay

Python standard library only, after copying this whole directory anywhere:

```sh
python3 -B validate.py --manifest
python3 -B test_offline.py
```

No Git, compiler, resident daemon, network, private scratch tree, native binary,
third-party package, or ambient HOME/PATH is required. The isolated test starts
Python with -I -S -B, empty HOME/PATH, and an audit hook that denies external
reads/listing, writes, subprocess/native execution, network and ctypes loading.
Six real denied probes are checked before replay. Archived drivers are inert
inspection evidence, never imported or executed by replay.

The validator extracts only selected original AST definitions from five exact
accepted Python oracles and the frozen candidate policy. It checks original
raw test tuples, diagnostics, recovery, source edit recipes, fresh runtime
nonces, triple flags, activation markers, exact commands, PID/startticks/exe/
socket/argv continuity, source63/TC4020 identity, byte/mtime touches, every first
touch, all samples, original-result medians and all stop receipts. It applies
accepted-to-v81, accepted-to-v82 and harness patches in memory and verifies
v82 changed only scheduling relative to v81. Fixed historical commitments are
separate from the attacker-resealable outer manifest. Negative tests rewrite
outer seals to exercise semantic rejection, not merely stale checksums.

## Actual independent execution

| Gate | Actual retained result | Root task |
| --- | --- | --- |
| Compiler and full native | PASS, 364.49s | 642585xyy3 |
| Named native tests | manifest 33, metadata 7, codegen 53, resident 373 | candidate-unit-v82 journals |
| Activation profile | 8 rows with current nonce and traces | 01548767jy |
| Strict OFF/ON/ON/OFF | 28 rows, original seven phases | 063334pbfu |
| Pinned factor | 54 calls, 20 timed, 24 runtime nonces | 369648jmee |
| Strict namespace cleanup | four validated stop0, stores retained | 7143478131 |

Both candidate arms fix RA_JIT_MANIFEST_CONTENT_GENERATION=1 and
RA_JIT_WATCHED_MANIFEST_BYTES=1. Only RA_JIT_PARALLEL_MANIFEST_PARSE changes
0/1. The reference has all three zero. Timed ON calls require exactly one
newline-terminated `ra-jit: parallel-manifest-parse-v81 batch` raw stderr line,
OFF/reference/version zero. This unchanged v81 contract proves actual bounded
batch selection, not useful parallel work or performance. V82 has a distinct
binary/source identity and atomic per-manifest dynamic scheduling.

Original native inventory has 3759 files; all runtime gates bind source63 and
TC4020. Full named journals and raw stdout/stderr are retained. Trace export
contains only declared resolver/clientplanning events and original full-trace
SHA/size, not unrelated trace contents. Root rustfmt parsing succeeded but
--check returned 1 for formatting-only changes. Source bytes were unchanged;
this was not a native failure. Private harness adapter explicitly binds
__file__ to pinned accepted Git authority without mutating frozen helper bytes.
No initial failed private-comparison artifact was supplied in the explicit
input set; no such adapter setup failure is represented as a native failure.

## Provenance and privacy

`private-inputs.json` records 490 explicit original input SHA256/size pairs,
rechecked unchanged after export. `export_private.py` declares allowed files,
JSON keys, process fields and trace events. Private path prefixes become stable
role tokens. No ambient environment, credentials, native binaries, arbitrary
recursive private directory copy, or private absolute paths are exported.
Original licenses are retained. Full source candidates, originals, authored 18
fixtures, ancestor patches and original oracle code are inspection evidence,
not shipped native implementation or a newly enabled suite mode.

`restoration/receipt.json` records preservation of all five live candidate
files against their frozen original SHA, exact accepted-native restoration and
removal of the two preserved new fixture modules, verified at
2026-10-31T17:45:29Z. This is point-in-time evidence only, not a statement about
subsequent native owners' work. No native source or live harness ships here.
`formatting/` preserves the unmodified historical formatting-only result.
`HISTORICAL.json` binds actual artifact bytes. `SHA256.json` covers the entire
portable bundle, including replay code. `seal.py` is an explicit maintainer
utility, not a semantic validator and not a way to redefine historical facts.
