# Native disk snapshot optimization, 2026-11-18

## Question and scope

Could a native on-disk compiler snapshot reach the fast edit loop of the Blaze-derived engine without replacing the rustc pipeline? This continues the first prototype rather than treating its neutral timing as the final answer.

All native builds, parser tests, compilation, runtime consumers and measurements below ran on the normal authenticated hosted guest `ip-172-31-47-214`, with its allocated affinity `0-15`. The physical pool host has more cores, but this account's guest allocation is 16. The production client and compiler defaults were not replaced. These are default-off compiler candidates, not published production performance.

## Two further implementations

1. **Checked single-pass AST decoding.** Validate every decoded span against the exact source allocation and retain the post-decode AST admission visitor. Remove the redundant full-tree re-encoding used to validate the previous decoder. Add optional phase timing separately from the existing hit/miss protocol. Commits `4482706ee` and `8af8756c5`.
2. **Restore before lexing.** Persist the actual successful native lexical stream's first-occurrence identifier effects as spellings and source spans, rather than session-specific symbol IDs. On a fully validated private cache hit, decode the AST and replay those effects before creating a fresh lexer or parser. Unsupported input, changed build/source identity, malformed data and corrupt/truncated records still fall back. Schema 2 binds the AST and lexical gallery in one integrity envelope. Commits `6c77abbeb` and `fb67f0976`.

The second implementation is a deeper boundary than merely making deserialization cheaper. An actual fresh-process hit now skips both lexing and parsing. It is still a bounded primitive parsed-AST snapshot, not a serialized expanded crate, HIR, type context, query graph, LLVM module or whole process.

## Correctness and proof of actual restoration

The checked-decoder native gate passed driver checking, 95 parser tests and executable doctests, followed by a matching compiler and standard-library build. The earlier-restore gate passed driver checking, 98 parser tests, executable doctests, and another matching compiler/standard-library build. A failed intermediate warning-denied gate is preserved separately rather than counted as acceptance.

Both frozen candidates passed the larger fixture's exact fresh/native status, diagnostics, fresh runtime consumer nonce, source-edit/error/recovery and genuine corruption/truncation controls. In the earlier-restore phase capture, unchanged and unrelated-helper-edit disk hits contain `gallery_replay`, with no `lex_parser` or `fresh_parse` phase. The compiler PID is fresh and bound to the native hit event. Thus these results are not a cache that silently misses or an accidentally reused live worker.

One illustrative unchanged-hit phase trace is: source admission 49.77 us, key 70.40 us, open 18.05 us, read 31.11 us, envelope 30.67 us, AST decode 260.74 us, post-decode admission 32.29 us, gallery replay 15.61 us. This is a diagnostic observation, not a statistical speedup result or a partition of total compilation time.

## Counterbalanced measurements

The exact same frozen Python harness, 193-extra-function generator, five independent samples, ten transition/control protocol, allocated affinity and instrumentation settings were used for all six captures. Capture order was original A, checked A, early-restore A, early-restore B, checked B, original B. Each capture uses fresh private stores. All six passed. `compare_reports` replayed the raw correctness/control gates, recomputed medians, and accepted the paired workload/host/harness identities. Compiler bytes may differ between candidates but remain identical before/after each capture.

Selected medians in milliseconds:

| Candidate/run | Unchanged, ordinary incremental | Unchanged, disk AST | Helper edit, ordinary incremental | Helper edit, disk AST | Root edit, ordinary incremental | Root edit, disk AST |
|---|---:|---:|---:|---:|---:|---:|
| Original A | 24.999 | 25.682 | 29.253 | 30.551 | 33.958 | 35.994 |
| Checked A | 28.286 | 28.735 | 35.247 | 35.057 | 39.120 | 40.346 |
| Early restore A | 37.445 | 36.749 | 43.256 | 44.111 | 45.869 | 47.588 |
| Early restore B | 40.658 | 37.298 | 43.712 | 45.759 | 50.344 | 50.810 |
| Checked B | 30.754 | 30.445 | 35.986 | 36.178 | 40.409 | 40.522 |
| Original B | 28.334 | 28.647 | 34.382 | 34.903 | 39.081 | 37.736 |

There is **no repeatable large end-to-end improvement and no 10x result**. New candidates are also slower in their disk-disabled controls, so cross-binary differences must not be attributed solely to snapshot decoding. Within each candidate, disk restoration remains near the ordinary incremental arm. This is a focused diagnostic, not the comprehensive session-weighted benchmark and not a default-promotion gate.

## What the measurements say about a deeper boundary

A separate original-compiler `-Ztime-passes` diagnostic on a 128-function fixture found warm incremental parsing around 0.66 ms, cached type checking around 0.35 ms, and a main compiler timer around 16.12 ms versus about 30.55 ms process wall time. Timings are nested and must not be added. Twelve checked-candidate `rustc --version` subprocesses had median wall time 11.209 ms. That command is only a startup diagnostic, not an estimate of all startup overhead in every build.

Eliminating sub-millisecond parse work cannot explain a tenfold reduction of this particular roughly 25-50 ms compile. The current incremental pipeline already restores some semantic query results from disk. A materially deeper design must change how early semantic invalidation is decided and which fresh-session setup/expansion/lowering steps can be avoided, not simply read this small AST payload with more cores.

This does not prove that persistent semantic state or disk-backed semantic snapshots cannot work. It distinguishes the tested parsed-AST boundary from that larger architectural project. Whole-process checkpointing was tested separately in `docs/compiler-process-snapshot-20261118.md`; it should not be confused with this native codec.

## Reproduction and original evidence

Frozen hosted roots:

- Original compiler: `native-disk-20261118-0932/compiler`.
- Checked candidate: `native-disk-deep-20261118-0951/compiler`.
- Early restore: `native-disk-gallery-20261118-1002/compiler`.
- Six raw captures: `native-disk-deep-comparison-20261118-1005/{old-a,checked-a,gallery-a,gallery-b,checked-b,old-b}/receipt.json`.

All roots are under `/home/ubuntu/.justrust/experiments`. Build scripts, successful and rejected gate logs, source hashes, compiler hashes, optional phase captures, raw receipts, native caches and comparison command are retained unchanged. The large original tar archive was uploaded in bounded lossless authenticated parts under `native-disk-deep-hosted-20261118-original-part-*`, with every part reconstructed and verified. `native-disk-deep-hosted-20261118-archive-manifest` binds their ordered SHA256 values and the aggregate archive SHA256 `27219d06665ab0e305182613f20ac99ff67869c22fd7dd76ef2f8bec9d71cfc1`. All 36 timing cells above were checked mechanically against the unchanged hosted receipts.
