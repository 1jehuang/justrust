# Resident diagnostic flat-text hash candidate (v23)

## Status: native gates passed, scoped experimental improvement

On October 30, 2026, root integrated the module and replaced only the old
allocated-text diagnostic hash call. Native gates passed. Acceptance is limited
to this experimental resident path and the warm selected fixture below. There
is no general build-speed claim or default promotion. No Cargo changes or
production counters were added by the helper.

The helper's integration destination is
`toolchain/resident/crates/ra-jit/src/diagnostic_hash.rs`. During the v22 freeze,
the original source-only candidate was staged at
`/home/jeremy/.jcode/scratch/blaze-native-engine-20261030/diagnostic-flat-hash-v23/diagnostic_hash.rs`.
The source-ready manifest in that directory records exact candidate hashes.

## Implementation and hash contract

`pub(crate) fn of(node: &syntax::SyntaxNode) -> u64` imports
`std::hash::BuildHasher` and calls `rustc_hash::FxBuildHasher.hash_one`.
For flat nodes, private `flat_text` borrows exactly
`tree.text()[usize::from(range.start())..usize::from(range.end())]`, where
`(tree, at) = node.flat()?` and `range = tree.range(at)`.
There is no scratch buffer, string copy, formatting, or syntax-text token walk
on this branch. The green fallback retains the exact old expression
`rustc_hash::FxBuildHasher.hash_one(node.text().to_string())`.
The intended contract is identical hash values, not a new hash format.

## Inspected API and borrowing shape

The actual fork API was inspected in
`toolchain/resident/vendor/rust-analyzer/lib/rowan/src/`:

- `api.rs`: `SyntaxNode::flat(&self)` returns
  `Option<(&std::sync::Arc<crate::FlatTree>, u32)>`.
- `cursor.rs`: `flat` matches by reference on the node representation and
  returns `&flat(root).tree` for flat nodes, or `None` for green nodes.
- `flat.rs`: `FlatTree::text(&self) -> &str` returns `&self.text`, and
  `range(&self, at: u32) -> TextRange` derives byte offsets.
- `api.rs`: `green(&self) -> &GreenNodeData`, `new_root(GreenNode)`,
  and `to_flat(&self) -> SyntaxNode` support explicit test representations.
- `cursor.rs`: `descendants()` includes each preorder enter event, including
  the source-file root.

The returned `&str` is expressed with the input node's elided lifetime. The
`Arc` is borrowed, not cloned or moved into a temporary owner. This shape is
based on the inspected source and subsequently passed native compilation.

## Differential test sources

Both module-local native unit tests passed in the integrated native binary.
Each walks every node subtree, including the root, for empty input, trivia-only
input with CRLF, UTF-8 identifiers/string/comment text, nested modules and
expressions, nested comments, empty blocks, and malformed recovery input.
They compare the helper to the old allocated-string hash and to an independent
original-source substring selected by `node.text_range()`. Text equality is
also checked so hash equality is not the only oracle.

The flat test uses explicit `to_flat()` to guarantee the positive branch
without depending on process-global representation state. It asserts
`flat_text` returns `Some`, compares exact text, and checks pointer and length
against the backing flat-tree slice. Recovery input must produce parse errors.
The green test constructs
`SyntaxNode::new_root(parsed_root.green().to_owned())` and verifies every node
has no flat representation before testing the fallback. Neither test toggles
`GREEN_TREES`, avoiding a new source of parallel-test interference.

## Native and scoped performance evidence

Root's frozen v23 gates passed: helper tests 2, diagnostic timing tests 3,
graph reuse 1, listed-root admission 1, and telemetry admissions 3. Full native
regression passed 351 tests, run `20261030-123822295-3023351`. The original
source-only manifest remains unchanged as a historical receipt, not a current
claim that native validation is pending. No helper Rust source changed after
its exact frozen materialization.

The first fixed-five-edit comparison did **not** clear the improvement floor:
raw engine median 108.325347 ms before versus 107.247475 ms after (about -1.00%).
Reference timing changed from 104.575675 ms to 129.957443 ms, confounding the
normalized -20.33% result. That normalized result is not accepted evidence of
a speedup.

Two extended interleaved diagnostics each passed 141 rows with 42 measured
edits per stack. Warm selected-fixture engine medians, in milliseconds:

| Ordering | Old hash | Flat hash | Blaze reference |
| --- | ---: | ---: | ---: |
| First interleaving | 106.2422585 | 93.0395020 | 99.9307425 |
| Independent reversed interleaving | 106.868752 | 100.779435 | 105.973125 |

These are descriptive warm-fixture observations, not a comprehensive
session-weighted benchmark or general/default-promotion gate.

A calibrated alternating matched profile passed 16 measured rows with
`RA_JIT_DIAGNOSTICS_STATS=1` set for both daemon startups and explicit
preparation runs. Raw worker deltas contain 373 selected-file callbacks per
row: old cumulative interval 7,627,533 to 9,105,572 ns, flat helper 326,941 to
506,480 ns. All reported transport dropped counts were zero. The initial
measured edit also reports a separate 231-callback file on both stacks, and
94 stale spans, which must not be silently merged into the selected-file
range. Callback intervals include source lookup, hashing and the selective
predicate, and are included in semantic time. They are not additive
critical-path savings or a measurement of hashing alone.

The original uncalibrated capture is retained as a failed attempt: it stopped
with `ValueError('missing positive diagnostics spans')` and zero accepted
rows. It is not a passing profile. Its raw first-row output and trace, runner
and state must remain alongside the calibrated evidence. Both attempts
record source/toolchain restoration and identity checks.

Scratch evidence roots are `blaze-native-engine-20261030`,
`resident-flat-hash-profile-v23-20261030`, and
`resident-flat-hash-profile-v23-calibrated-20261030` beneath
`/home/jeremy/.jcode/scratch/`. Portable evidence followthrough belongs under
`bench/results/resident-diagnostic-flat-hash-20261030/`, retaining raw native
logs, both interleaved runners and rows, fixed-five results, original failed
capture, calibrated traces/worker deltas, and frozen source/binary identities.
That archive must independently validate raw markers and hashes, not merely
repeat these summary numbers. No new native workload is required for archival
validation.
