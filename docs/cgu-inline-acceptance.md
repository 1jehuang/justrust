# CGU shared-inline traversal acceptance

`bench/reuse-cgu-inline-accept.py` is a correctness gate for default-off,
tracked `-Zjr-cgu-shared-inline-traversal=yes/no`. The candidate shares the
reachable `LocalCopy` traversal visited set per CGU rather than repeating the
same DFS independently for every global root. Semantic equivalence, including
symbol ownership and cyclic reachability, is the contract. This is not a
speed benchmark, traversal-hit proof, or default-promotion gate.

## Execution boundary

Only mocked Python tests may run before root releases its frozen compiler:

```sh
python3 -m unittest discover -s bench -p 'test_reuse_cgu_inline_accept.py' -v
```

Root owns building/freezing the compiler, native acceptance, and representative
performance measurement. Do not execute the native harness or its fixture with
an old or installed compiler while waiting for release. After release:

```sh
python3 bench/reuse-cgu-inline-accept.py \
  --rustc /absolute/path/to/frozen/rustc \
  --output /absolute/path/to/new-cgu-inline-acceptance --require-mechanism
```

The output directory must not exist and its parent must exist. Pass the actual
compiler executable and its sysroot exposing the driver under
`lib/*rustc_driver*`, not a wrapper. No installed compiler is replaced and
`RUSTC_BOOTSTRAP` is not set by this harness. Initial implementation used only
mocks. After root released a frozen compiler, root ran the full initial gate
and authorized the focused fixture repair described below.

## Configuration and revision matrix

The Cartesian product contains eight configurations:

- Codegen units: `1` and `16`.
- Incremental: enabled and disabled.
- Optimization: `0` (debug-like) and `2` (release-like), with debuginfo `0`.

These are direct rustc settings, not Cargo profile names. Every compile requests
`-Zincremental-verify-ich`, including the nonincremental configurations, which
omit `-C incremental` entirely. No inherited environment settings are used to
silently enable incremental output. Configuration-specific cache namespaces
prevent incompatible settings from sharing incremental directories.

Twelve ordered source revisions run in every configuration:

1. Shared diamond graph initially.
2. Unchanged rebuild.
3. Shared leaf body edit, changing all root results.
4. Leaf recovery.
5. One global root body edit, leaving other roots unchanged.
6. Bounded recursive cycle input edit.
7. Added generic drop instantiation with observable drop-count change.
8. Duplicate `no_mangle` symbol error.
9. Duplicate-symbol recovery.
10. Type error in shared leaf.
11. Error recovery.
12. Unchanged recovery.

Each revision runs seven separate compiler processes:

1. Fresh `no` in a revision-specific empty cache.
2. Fresh `yes` in a separate revision-specific empty cache.
3. Persistent `no`.
4. Persistent `yes`.
5. Transition `no` before.
6. Transition `yes`.
7. Transition `no` after.

Persistent no/yes directories are distinct and retained across **all revisions**,
including failed compilations and recovery. Transition modes share exactly one
additional directory per configuration. Because the option is tracked, a flag
transition may invalidate incremental state. Persistent same-flag revisions
are independently tested and cannot be replaced by transitions. In
nonincremental configurations these seven modes remain repeated direct
compilations, not claims of actual cache persistence or reuse.

This yields **12 revisions x eight configurations = 96 revision/configuration
cases**, or **672 main compile observations**. No dependency compiler calls are
needed for this standalone fixture.

## Graph and observable semantics

Three externally named, `inline(never)` global roots live in two modules.
Two roots share module `a` and a third lives in module `c`.
All call the same inline diamond: left and right each call the shared leaf.
This gives both shared root reachability and a repeated descendant within the
diamond. The two same-module roots share an initial CGU before final merging,
which is necessary to exercise cross-root suppression. Final codegen-units=1
does not itself put different-module roots in the same initial traversal CGU.
The third, module-separated global root exercises CGU isolation under the
16-CGU setting, without asserting that every module must occupy a distinct
final CGU. The one-CGU configuration independently stresses root sharing inside
a single unit.

Two inline functions form a recursive call-graph cycle. Both have a zero-depth
base case and decrease the depth on every call. Runtime invokes depth `4`, then
`6` in an edit, so this does not require unbounded recursion. The fixture also
instantiates a generic inline identity for `u32` and `u64`, and a generic `Drop`
implementation for two concrete types. An atomic drop counter is read after
both values leave scope. The generic/drop edit adds a third concrete type and
checks the changed drop count independently.

Two additional public functions in different modules use the same
`#[unsafe(no_mangle)]` name in the duplicate-symbol error revision. The gate
requires ordinary error status `1`, exact diagnostics across modes, and valid
recovery. A false success or compiler crash does not satisfy the error case.
This ensures symbol diagnostics cannot be silently lost during deduplication.

## Exact differential and identity contract

Within each configuration and source revision, fresh `no` is the reference.
Every other mode must match its exact compiler exit status/stdout/stderr bytes
and, on successful compilation, runtime exit status/stdout/stderr bytes.
Every successful mode also checks an independently specified expected runtime
output. There is no path normalization, newline conversion, span filtering,
warning suppression, or diagnostic sorting. Raw non-UTF-8 bytes are retained
as base64. Artifact SHA-256 values are recorded for investigation but need not
match. Each configuration is compared with its own fresh reference, not with
another optimization or CGU setting.

All modes use the same absolute source path, executable output path, crate
name, and edition. Only tested settings and cache paths vary. Old executable
output is deleted before every compile, preventing stale execution after an
error. Expected errors must exit `1`, not ICE, crash, or falsely succeed. The
harness imports the existing byte-exact identity/validation/probe helpers in a
private module instance, without modifying any existing harness.

Before and after the run, record compiler executable and resolved driver
library SHA-256 identities, raw `-vV`, and the sysroot query. Any changed identity
invalidates acceptance. Both explicit flag values and verify-ICH are probed
before fixtures. Probe rejection or crash fails with raw diagnostics, never
skips a required case or substitutes a compiler.

`acceptance.json` is written even after a failure, retaining attempted mode
results, full source revisions and expected outputs, configuration settings,
commands, raw diagnostics, completed/planned case counts, error details, and
available before/after identities. Mock tests validate only the harness
contract. Native root gates are still required before source promotion, and
performance claims require separate measured before/after workloads.

## Independent positive native mechanism gate

Root's recommended command includes `--require-mechanism`. The semantic gate
alone records `mechanism_verdict=not-requested` and must not be presented as
proof that the new traversal executed. A requested mechanism failure preserves
a passed `semantic_verdict` but fails the overall run.

After all 672 semantic observations, a separate four-process proof uses the
shared-diamond fixture with one CGU, incremental enabled, and optimization `0`:
cold off, cold on, unchanged warm off, unchanged warm on. Off/on caches are
distinct, retained across the warm step, and independent of every semantic
cache. Every compile/runtime must still match exactly, and runtime expectations
are checked independently. Each process has a unique retained profile directory
and explicitly requests `-Zself-profile-events=generic-activity`.

The compiler owner audited the fixture shape: the entire diamond chain uses
`#[inline(always)]` so nodes are `LocalCopy` even at optimization `0`. Ordinary
`#[inline]` becomes globally shared at that setting and is not a substitute.
The normal MIR inliner is disabled for this opt0 incremental configuration,
while mandatory MIR ForceInline applies to the distinct internal force-inline
attribute, not `inline(always)`. Thus the candidate graph remains available
for the CGU traversal proof without adding a separate inliner-control flag.

Only exact resolved `GenericActivity` interval labels count:

- `jr_cgu_inline_shared_traversal`: enabled shared traversal scope.
- `jr_cgu_inline_shared_revisit`: duplicate `LocalCopy` reachability suppresses
  a repeated DFS.

Both off profiles must have zero events for both markers. Enabled profiles in
aggregate must have positive scope **and positive revisit** counts. Flag
acceptance or scope entry alone is insufficient. A profile with revisits but
no scope is rejected. An unchanged warm partitioning query may be green and
emit no new markers, so either cold or warm can establish the positive proof.
Counts, not nonzero elapsed durations, establish actual branch execution and
duplicate graph-work avoidance. They do not establish a speedup.

There must be exactly one `.mm_profdata` in each profile directory. Reports
retain its SHA-256, byte size, metadata, marker/record counts, and unresolved
record count. Unknown virtual query labels remain unknown, never guessed into
one of the static markers. Malformed profiles, wrong event kind/type, absent
markers, incomplete sequences, or nonzero off controls fail the requested
mechanism gate. All observations remain in `acceptance.json` on failure.

## Native fixture calibration, October 30, 2026

The initial full run passed all 672 semantic observations but correctly failed
the independent mechanism proof: enabled cold and warm profiles each recorded
23 traversal scopes and zero cross-root revisits. All three roots originally
lived in different modules, so traversal ran with separate initial CGU visited
sets. Final one-CGU merging happens afterward and cannot create cross-root
reuse during the earlier traversal. This was a fixture limitation, not reason
to weaken the positive proof or change native marker emission.

The repaired fixture moves root_b into module `a` beside root_a while retaining
root_c in module `c` for isolation. A root-authorized focused four-process run
against the same immutable compiler passed exact diagnostics/runtime and the
unchanged proof: cold off `0/0`, cold on `23/1`, warm off `0/0`, warm on `23/1`
(traversal/revisit counts). Compiler-driver SHA-256 remained
`29ea709935d751ce13e77995173659006c1e15683e0f0645616d12cd5a3a1665`.
The original failed report remains at
`/home/jeremy/.jcode/scratch/cgu-inline-cache-20261030/acceptance/acceptance.json`.
The focused repair report and its four raw profiles remain under
`/home/jeremy/.jcode/scratch/cgu-inline-cache-20261030/mechanism-repair-v2/`.
The focused report is `focused-evidence.json`. These scratch paths are an audit
trail, not a portable retained-results bundle. The repaired full matrix remains
root-owned and must pass before source promotion. No speedup is inferred.
