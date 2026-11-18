# Native disk AST snapshot experiment, 2026-11-18

## Answer

Compiler-native disk restoration works on the normal authenticated hosted guest,
without CRIU, root, or kernel checkpoint support. This first implementation
restores a bounded, pre-expansion parsed AST, not a complete compiler process or
its typed semantic state. It did **not** produce a repeatable improvement over
ordinary disk incremental compilation on this workload. A live worker remained
faster. Giving the experiment 16 CPUs instead of one did not turn this small
snapshot into a substantial performance win.

This is a scoped diagnostic, not a production/default promotion, a 10x claim, or
an estimate of the ceiling of richer semantic snapshots. The comprehensive
session-weighted synthetic gate was not run and no default was changed.

## What was implemented

`rustc_parse::disk_ast` is default-off. An explicitly selected private directory,
`JUSTRUST_RUSTC_DISK_AST`, enables it for ordinary file-input compilation. The
native interface routes through the existing effect-replaying parser entry:

```text
load source -> fresh lexing/parser construction -> validate native disk AST
            -> restore admitted AST OR parse normally -> expand/lower/analyze/codegen/link
```

Lexical diagnostics and symbol-gallery effects still run fresh. Reuse is limited
to the existing bounded, effect-free AST subset: unloaded module declarations
and primitive single-expression functions. Attributes, macros, lazy tokens,
unsupported syntax and effectful/recovered parses fall back normally. This is not
an expansion, type-checking, borrow-checking or codegen snapshot.

The native AST codec encodes symbols as text and re-interns them in the new
process. Its span encoder checks every encoded span for root hygiene, no parent,
and membership in the exact source file, or a dummy span. Positions are not
relocated. Exact normalized source, normalization information, filename, edition,
strip mode and absolute source start position participate in the key.

The candidate embeds an explicit 64-hex source/configuration digest through
`JUSTRUST_NATIVE_COMPILER_SOURCE_HASH`, alongside `CFG_VER_HASH`, native version,
codec source and schema. Missing build identity disables reuse. The hosted build
computed this digest from compiler/library/bootstrap source and bootstrap/Cargo
configuration, excluding Python bytecode caches. It was
`c16c107cef7275aa4dd539ad4e8e6a447937ddf9d86c7bbc1dd4860316d91549`.
The large driver is **not** rehashed by every fresh compiler invocation.

Persistence is bounded to 32 direct-mapped records, each at most an 8 MiB payload
plus envelope, and one staging record. Private owner/mode checks, pinned directory
FDs, no-follow file operations, nonblocking locking and atomic rename protect
publication. The integrity envelope validates schema, key, lengths, checksum and
native footer before decode. Checked string decoding validates UTF-8, sequence
lengths are bounded, unsupported session IDs reject, and the restored AST is
admitted and span-checked again. IO failure or corruption falls back to parsing.
This is same-user private compiler scratch, **not** a cache format authorized for
untrusted or multitenant artifact publication. Its checksum is not authentication
against an owner deliberately forging their own cache.

Telemetry is native opt-in JSONL `{event,pid}` through
`JUSTRUST_RUSTC_DISK_AST_STATS`. Positive hit evidence must match a fresh compiler
PID. No wrapper parses stderr to infer snapshot hits.

## Actual hosted execution

All native checks, tests, builds and compiler measurements ran in the normal
account-authenticated hosted guest, hostname `ip-172-31-47-214`, kernel `6.1.155+`.
The physical production machine has 96 cores, but this guest's allocated share is
16 CPUs. Runs explicitly used affinity `0` or `0-15`; the receipt records affinity
and the root-owned `/run/pool/share` file. No privilege or production-host
reconfiguration was used. The production host was not stopped.

The hosted typecheck passed. All **93 parser tests** passed, plus the executable
parser doctests. The matching compiler and complete standard library built
successfully. The first real acceptance run used zero extra functions. Four
measurement runs then used 64 extra primitive functions and five independent
samples each, in run order `16-a`, `1-a`, `1-b`, `16-b`.

Each sample rotates four same-compiler modes across ten source transitions:

- fresh process without incremental state;
- fresh process with ordinary `-Cincremental` disk state;
- fresh process with its own incremental state plus the new disk AST cache;
- live owned worker with the existing scoped RAM AST cache.

The live arm does not have incremental enabled, so these are complete mode
comparisons, not isolated estimates of the cost of RAM versus disk storage.
Compiler command timings exclude the fresh consumer compilations and executions.
Initial compilation and worker startup are separate from the warm table below.

The transitions cover unchanged input, helper-only edits, root edits, repeats,
syntax and type errors, and recovery. Every arm must match the exact fresh status
and diagnostic streams. Every successful revision gets a freshly compiled
consumer with a unique runtime nonce and exact arithmetic outputs. Fresh-process
PIDs must differ and the live worker PID must stay fixed. Corrupt and truncated
private snapshots must compile successfully via fallback, without a false hit.
Medians are omitted if any gate fails. All five captures passed. There were
**840 compiler observations and 84 corruption-control compilations**, including
the initial diagnostic capture; counts are independently derived from the
archived original receipts.

## Results

Warm medians, milliseconds, 64 extra functions. Each row is the median of five
samples within its named run. These are descriptive targeted timings from the
same development candidate, not published-client end-to-end timings.

| Affinity/run | Transition | Fresh | Ordinary incremental | Disk AST + incremental | Live AST worker |
|---|---|---:|---:|---:|---:|
| 16-a | unchanged | 31.947 | 24.022 | 22.711 | 14.761 |
| 16-b | unchanged | 34.147 | 24.661 | 24.922 | 14.169 |
| 1-a | unchanged | 33.594 | 24.355 | 24.580 | 14.841 |
| 1-b | unchanged | 33.605 | 24.325 | 24.498 | 14.813 |
| 16-a | helper edit | 33.737 | 30.992 | 29.535 | 13.994 |
| 16-b | helper edit | 33.984 | 29.938 | 30.478 | 14.043 |
| 1-a | helper edit | 33.659 | 31.541 | 31.435 | 14.814 |
| 1-b | helper edit | 33.759 | 31.362 | 31.380 | 14.906 |

The first 16-CPU run's modest favorable disk-cache differences did not reproduce
in the second. Single-CPU unchanged medians were slightly worse with disk AST
restoration. Root edits and error/recovery timings are also preserved in full,
not excluded to improve the headline. The positive genuine disk-hit requirement
passed, so the neutral outcome is not simply a cache that never restored data.

The successfully retained 64-function root snapshot was 18,451 bytes. The raw
archive also contains zero-length deliberately truncated records from the
fail-open controls. Restoration in this prototype is serial, and it re-encodes
the decoded AST to validate every span. The affinity experiment does not
implement or measure parallel deserialization. There is no independent IO/decoder
phase attribution, and the filesystem is warm. These measurements therefore do
not prove that storage itself is the bottleneck, or that large typed snapshots
cannot benefit from a parallel decoder or a different workload.

## Immutable identities and original evidence

The hosted compiler was copied to an immutable experiment sysroot, excluding only
rust-src links. All four arms use those same bytes, verified before and after:

- Launcher SHA256: `806d6f0d9cc027c77d63343c0ab660a6a8da91df5c05d91e454b5051dd2e499e`
- Driver SHA256: `9c8a8d6d61803826a993aec40473cba73281fb1dd44710602a609f2e4940b98a`
- Guest experiment: `/home/ubuntu/.justrust/experiments/native-disk-20261118-0932`
- Untouched receipt paths: `diagnostic/receipt.json` and
  `measure-{16-a,1-a,1-b,16-b}/receipt.json`

The lossless account-private record
`bench/native-disk-snapshot-hosted-20261118-0933-originals` contains exact original
receipts, native cache files and the exact executed Python harness/helpers. Its
340,791-byte tar.gz SHA256 is
`08c6ad8fe24cf03caa67e876ab81d868085bfdf11030b34c5e16dd651dc2c6c1`.
`bench/record_archive.py put` authenticated a readback and verified exact binary
reconstruction. Receipt JSON was not reformatted before archival.

The native parser/build gate output is separately archived as
`bench/native-disk-snapshot-hosted-20261118-native-gate`.
The harness has 21 passing Python mock tests. Implementation commits include
`4c68a27c4`, `9a4417b49`, `de3aa0562`, `fd150fcc1` and `182ee2436`.

## Architectural implication

Disk serialization is feasible without privilege, and this prototype genuinely
reconstructs native data in a new process. However, saving only a small parse
boundary leaves process startup, lexing and the rest of the compiler pipeline.
The experiment does not justify replacing resident workers, enabling the feature
by default, or promising a 10x improvement. A richer snapshot would need to skip
measured dominant semantic work, while reconstructing interner, source, DefId,
query and allocation identities correctly. That is a distinct implementation and
acceptance gate, not a conclusion supplied by this parsed-AST experiment.
