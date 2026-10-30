# Empty lint-level owner setup experiment

## Status

Default-off experiment with a passed focused native correctness gate. After the
scalar baseline was stopped, the owned stage1 compiler and matching library,
rustdoc and Clippy were rebuilt. All 22 cases passed off/on with fresh and
incremental caches, totaling 88 compiler invocations with fingerprint verification.
Exact sources, commands, diagnostics and artifact hashes are retained in
`bench/results/query-value-retention-20261030/lint-native-accept/acceptance.json`.
The compiler executable SHA-256 is
`570faeb27338cea8625c4a4a7eaeef71fc75470508657ff489c3da96ce5f9144`,
and its driver SHA-256 is
`c7b44ed95c8f59faaf47be89a72db8a4445e9ff1595f5c458afa9890aff72fa5`.
The executable hash alone does not identify compiler implementation changes:
the driver differs from the frozen scalar experiment. Performance measurements
and broader regression coverage remain pending. No speedup claim or default
promotion is justified by this finite correctness gate.

## Hypothesis and implementation

`-Zjr-empty-lint-level-fast-path=yes` is an independently tracked session
option, default false. It does not enable the small-query-value experiment.
In `shallow_lint_levels_on`, fetch `tcx.hir_attr_map(owner)` and, only when the
flag is enabled, the owner is not the crate, and the entire owner attribute map
is empty, return `ShallowLintLevelMap::default()` before fetching the lint store,
features, or registered lint tools. No HIR traversal is removed: the existing
provider already skips traversal when this map is empty. This experiment only
removes unnecessary setup/query dependencies on that branch.

With the flag off, all existing setup and dispatch still execute. Attribute-map
fetch now precedes store retrieval. Crate owners never take the shortcut:
`add_command_line` and crate attributes retain their existing processing.

## Source-level correctness argument and remaining risks

* `rustc_middle/src/lint.rs` defines the shallow map as default-empty specs and
  expectations. The old empty-attribute branch cannot insert either, except for
  crate command-line specs, which are explicitly excluded.
* Effective inherited lint levels are not stored in this map.
  `ShallowLintLevelMap::probe_for_lint_level_spec` walks HIR parents and loads
  their owner maps. Returning an empty child map does not suppress crate/parent
  forbid, deny, expect, warnings-group levels, or command-line settings.
* `hir_attr_map(owner)` is still read inside the query. Its dependency captures
  empty/nonempty edits, body-local attributes, and macro-generated attributes.
  The guard tests the whole map, not only attributes attached to the owner's
  root node. Owners without root attributes can still have body attributes.
* Expectation IDs and records are created by attribute handling. No attributes
  means no locally introduced expectations. Ancestor expectations remain in
  ancestor maps and keep their normal fulfillment path.
* Features and registered tools are consumed by lint attribute processing in
  this builder. The empty branch never processes attributes. The builder has
  no drop hook whose effects would need preservation. Store retrieval is an
  accessor, not lint registration.
* The flag is `[TRACKED]`, so off/on compiler sessions cannot silently share
  incompatible incremental assumptions. The map query's reduced dependency
  set when enabled is intentional. Fresh-versus-incremental comparisons still
  need to validate this argument with the native compiler.
* Moving or omitting query evaluation can change diagnostic timing or expose
  query-cycle assumptions even when the returned map is identical. Source
  inspection is not proof of diagnostic equivalence. Native acceptance must
  pass before enabling this in measurements or deploying it.

## Focused acceptance harness

`bench/reuse-lint-accept.py` drives the newly built compiler directly, without
wrappers or caching services. It compiles one stable source pathname through
22 ordered edits. Every edit runs off/on with separate persistent incremental
caches, and off/on with fresh caches. Each compilation checks expected status
and diagnostic codes, then compares complete parsed JSON diagnostic trees
(including spans, suggestions, children, and rendered text) across all four
runs. The fresh reference catches stale results even when both incremental
modes fail identically. Unknown flags or non-JSON compiler errors fail closed.
Every invocation also enables `-Zincremental-verify-ich`. With `--output`, the
harness retains an `acceptance.json` with exact compiler executable/driver
hashes, compiler version and sysroot queries, sources, commands, unfiltered
diagnostics, expected outcomes, completed case count and verdict. A partial
report is kept on failure and printed in full. The output directory must be
new, with an existing parent, and is created with mode 0700. Without `--output`,
temporary evidence is removed when the command exits.

Coverage: empty owners, crate/parent forbid, command-line deny, attribute
addition/removal, body-local attributes, fulfilled/unfulfilled and inherited
expectations, nested function owners, illegal forbid lowering, macro definition
edits with generated attributes and reversion, unknown lint diagnostics,
type and parse errors followed by recovery. This is a focused semantic gate,
not general Rust regression coverage or a test of proc-macro execution, native
linking, custom lint drivers, or all query-cycle/error-ordering possibilities.

The harness uses metadata output to exercise lint analysis without linking.
Its fixture expectations passed against the native off build and all three
other configurations. This finite result does not establish equivalence for
all Rust programs or all diagnostic ordering interactions.

## Reproducing the native gate

From the repository root, with no overlapping timed measurement:

```sh
# Build native compiler and matching sysroot/tools.
(cd toolchain/rustc && ./x build --stage 1 compiler/rustc library src/tools/rustdoc src/tools/clippy)
python3 bench/reuse-lint-accept.py toolchain/rustc/build/host/stage1/bin/rustc \
  --output "$HOME/.jcode/scratch/query-retention-bench-20261030/lint-native-accept"
```

For a release-quality performance comparison, build the same optimized mode
for baseline and experiment and freeze both outputs. Use identical scalar flag
settings and all other compiler settings on both sides. This patch has no
claim about query-value retention. Run relevant native lint/UI regressions and
representative cold/incremental debug/release builds before promotion. Follow
the root benchmark rules for pinned comprehensive before/after comparisons,
including command/change regressions. Do not substitute this harness for the
session-weighted performance gate. Do not install this experimental compiler
over the shipping justrust client.

Safe during the measurement window:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s bench -p test_reuse_lint_accept.py -v
```
