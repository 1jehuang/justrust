# Standalone object-layout calculator receipts

Private source checkpoint: `5633a107b450f8051141b0ef4fd6a2c66ba4dc30`, based on
preserved `837c6a941` and `885ccc768`. This is an isolated building block, not
source Rust layout-intrinsic, addressed-storage, graph-fact, FFI-call or Box
acceptance. No compiler registration, node ID, backend/IR activation or nominal
key changes occurred. HEAP-LAYOUT.md was preserved unchanged.

## Exact commands and placement

| Run | Command (stable justrust client) | Placement | Outcome / source state |
| --- | --- | --- | --- |
| `20261018-215233513-848421` | `justrust check --remote --test object_layout` | hosted, forced remote | exit 0, before final forged-key hardening |
| `20261018-215402698-929520` | `justrust test --local --test object_layout -j 2 -- --test-threads=1` | local, explicit coordinator release for C controls | 14 passed, before final hardening |
| `20261018-215404348-930430` | `justrust test --local --test graph_integrity -j 2 -- --test-threads=1` | local, same released bounded window | 7 passed, unchanged ratchets |
| `20261018-215542061-994215` | `justrust test --local --test object_layout -j 2 -- --test-threads=1` | local, released bounded window | final hardened source, 16 passed including C controls |
| `20261018-215542788-994408` | `justrust test --local --test graph_integrity -j 2 -- --test-threads=1` | local, released bounded window | final hardened source, 7 passed, unchanged ratchets |
| `20261018-215908225-1061163` | `justrust clippy --remote --test object_layout -- -D warnings` | hosted, forced remote | final source checkpoint, exit 0, no warnings |

`justrust show` confirmed `remote on hosted` for check and clippy, and
`local (JUSTRUST_REMOTE=off)` for the runtime/graph controls. Hosted used
`nightly-2026-10-08`, local used `jr-ac20de5e7921`. The hosted clippy log confirms
`clippy-driver` checked the integration-test target even though the recorder
reported zero rustc compilation units. No local runtime repeats occurred during
the subsequent central full-suite window.

The first five runs preceded adding the new files to Git, so their Git metadata
reports the base checkpoint with zero tracked modifications. They did exercise
the new files, including the final two added forged-key/tamper cases in the
16-test run. Final tested/committed source hashes:

- `src/compiler/object_layout.rs`: SHA-256 `ae98c6d1e91319cac08f1d65301815c663e00a45fa42742c808fb2b9b241c60b`
- `tests/object_layout.rs`: SHA-256 `b0b9aa5381fb04960ebd18d1da9421f8009db995bc81654594d6e3a8332b3b41`

## Evidence scope

The calculator admits a private closed grammar of 12 integers, f32/f64,
bool/char/unit, four thin-pointer kinds and fixed arrays. Complete exact keys,
target/version, byte receipts, wrong authority/type, malformed encodings,
forged key bounds, child/stride/validity tampering, overflow versus excessive
object size, sticky work/bytes/depth exhaustion, ZSTs and empty-array alignment
have controls. Pointer grammar is checked completely while pointer layout does
not calculate its pointee layout, including an overflowing array pointee.

Bootstrap Rust size/alignment controls cover all scalars and zero/nonzero/nested
arrays. A C11 executable controls 17 scalar/pointer storage shapes and their
three-element array size/alignment/stride. C uint32_t controls Rust char's storage
only, not Unicode validity. C has no standard unit/ZST control. These are not
bidirectional native-compiler memory reads/writes or platform calling-ABI tests.

The existing graph integrity suite passed, but this module is unregistered, so
it does not prove new graph-fact shape, dependency or source-authority behavior.
Reachability of the new standalone output was manually reviewed through
ClosedTypeKey/RetainedStorage/immutable PlanAuthority; no output retains a budget,
ledger, meter or compiler Ty. The immutable receipts are accounting evidence,
not canonical source admission or exact invocation authority. Hare reviewed the
initial boundary and identified the calculate-side forged-key hardening. The
final gates were run by this lane, not independently replayed by hare.

No full native compiler suite, performance benchmark or speedup claim follows.
Source-authenticated queries, typed graph registration, real addressed storage,
nominal layout and pointee cleanup remain separate approval-gated work.

## Export provenance

`2026-10.jsonl` contains exactly six owned records from stable-client
`justrust export`, run from the worktree root with an isolated scratch output.
Only the exact IDs above and this lane's agent session were selected. All
unrelated exported histories remain outside the checkout. The export's private
path/argument redaction is preserved, so the command and placement annotations
above supply the explicitly verified scoped context. No records or results were
synthesized, and no unrelated files were committed.
