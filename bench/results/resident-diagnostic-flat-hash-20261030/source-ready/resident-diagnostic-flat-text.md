# Resident diagnostic flat-text hash candidate (v23)

## Status: source only, native and performance gates pending

This candidate is not integrated and has not been compiled or run. No speedup
or native correctness result is claimed. Root must finish the frozen v22 native
build/timing work before integrating or testing v23. This work does not change
`main.rs`, `Cargo.toml`, diagnostic call sites, or production counters.

The helper's integration destination is
`toolchain/resident/crates/ra-jit/src/diagnostic_hash.rs`. During the v22 freeze,
the candidate is staged at
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
based on source inspection only. Compiler validation remains pending.

## Differential test sources

Two module-local native unit tests are supplied, but have not been executed.
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

## Required integration gates

After v22 is released, root must copy the staged helper into its destination,
wire the module and diagnostic call site, and compile and execute the native
differential tests. Those operations are deliberately outside this helper's
ownership. Native lifetime/type validation and diagnostic behavior checks are
still pending. Performance measurement must compare appropriate before/after
workloads under the project benchmark policy. Source formatting and diff
checks alone establish neither correctness nor a performance win.
