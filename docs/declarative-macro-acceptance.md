# Native declarative macro cache acceptance

`bench/reuse-declarative-macro-accept.py` is the correctness gate for the
forthcoming default-off `-Zjr-cache-declarative-macros=yes/no`. It follows the
native dead-code retention transition harness, but exercises declarative macro
expansion. It is not a speed benchmark or evidence of a cache hit.

## Execution boundary

Only mocked Python unit tests were run during harness implementation. Root
subsequently released and executed the frozen compiler on October 30, 2026,
with retained results described below.
Do not run this harness, build rustc, or execute native fixtures until the root
coordinator releases the frozen new compiler. The root owns that build, native
acceptance execution, and the larger performance measurement. Never substitute
an old compiler or silently fall back to another flag.

Mock-only validation, safe before compiler release:

```sh
python3 -m unittest discover -s bench -p 'test_reuse_declarative_macro_accept.py' -v
```

After root releases the frozen compiler, run from the repository root:

```sh
python3 bench/reuse-declarative-macro-accept.py \
  --rustc /absolute/path/to/frozen/rustc \
  --output /absolute/path/to/new-declarative-macro-acceptance \
  --require-native-hit --require-tt-hit
```

The output directory must not exist and its parent must exist. Pass the actual
compiler executable, not a shell wrapper. The compiler's sysroot must expose
its `lib/*rustc_driver*` shared library. No installed compiler is replaced.
The harness uses the compiler's own sysroot and does not set `RUSTC_BOOTSTRAP`.

## Differential contract

Each ordered source revision runs these seven modes:

1. Fresh `no` reference in a revision-specific empty incremental directory.
2. Fresh `yes` reference in a different empty incremental directory.
3. Persistent `no`, retaining its cache across revisions.
4. Persistent `yes`, retaining its cache across revisions.
5. Transition `no` before.
6. Transition `yes`.
7. Transition `no` after.

The last three modes share exactly one incremental directory, retained across
all revisions. Dependency and executable caches occupy separate subdirectories
within each mode's cache. Every native fixture compile, including dependency
compiles, requests `-Zincremental-verify-ich`.

All modes compile the **same absolute source path**, with the same crate name,
edition for that revision, output executable path, dependency source path, and
dependency artifact path. Fresh references do not live in copied source trees.
Only the incremental directory and the explicitly tested flag vary within a
revision. Old executable and dependency outputs are removed before rebuilding
so a failed compile cannot run a stale binary. The dependency is rebuilt in
each mode before compiling the consumer.

Against the fresh `no` reference, every other mode must match:

- Compiler exit status, stdout bytes, and stderr bytes.
- Dependency compiler exit status, stdout bytes, and stderr bytes, when present.
- Executable exit status, stdout bytes, and stderr bytes, for successful cases.

Every mode also checks the fixture's independent expected runtime output or
expected ordinary compiler-error status `1`. A crash, ICE exit status, false
success, or runtime mismatch cannot masquerade as an expected failure. Artifact
SHA-256 values are recorded for investigation, not required to be identical.

**Nothing is normalized.** No path replacement, span relocation filtering,
ANSI stripping, newline conversion, warning elision, or diagnostic sorting is
allowed. Reports store subprocess stdout/stderr as base64 so non-UTF-8 bytes
remain lossless. Commands and elapsed times are evidence, not compared output.
If a differing incremental cache path appears in diagnostics, it is currently
a failure, not a blanket normalization exception.

## Revision matrix

The ordered matrix includes unchanged rebuilds and final recovery, plus:

| Family | Coverage |
| --- | --- |
| Eligible literal subset | Isolated `() => { 42 }`, unchanged rebuild, and unrelated same-width body edit, without metavariables or parser fragments |
| Definition/input | Macro body change and invocation argument change |
| Ordinary body | Non-macro function body edit alongside unchanged invocations |
| Scope | Nested local macro shadowing with an edited inner definition |
| Hygiene | Same-spelled caller and transcriber-local identifiers |
| `$crate` | Exported dependency macro resolves its defining crate's helper, including dependency rename and caller-local shadow |
| Dependency edits | Exported macro definition edit, helper body edit, failed expansion and recovery |
| Spans | Relocated definitions and invocation lines, with warnings kept exact |
| Diagnostics | Unused macro rename, unused rule edit, formerly unused rule invoked, deny and recovery |
| Expansion errors | Invalid macro input and subsequent valid recovery |
| Interpolation | Opaque forwarded `expr` versus direct literal matching |
| Editions | 2021 versus 2024 `expr` const-block acceptance, 2018 versus 2021 or-pattern acceptance |
| Tracing | Trace-enabled full differential matrix if supported |

The native implementation may initially cache only a narrower safe subset.
This does not remove any correctness cases. Unsupported invocation shapes must
fall back without observable differences, including unused-rule accounting,
hygiene, diagnostics, and errors. The literal nested invocations also provide
simple cache-eligible candidates without depending on interpolation support.
The semantic matrix does not infer eligibility from a successful build. Native
eligibility and reuse require the separate marker gate below.

## Positive native reuse evidence is a separate gate

Semantic equivalence alone is not proof of a cache hit. The recommended CLI
above includes `--require-native-hit`. Without it, the report explicitly sets
`native_hit_verdict` to `not-requested`, even when `semantic_verdict` is `passed`.
Never promote a semantic-only pass as evidence that the native path was exercised.

With the option enabled, a separate literal-only fixture contains exactly one
`macro_rules! literal { () => { 42 }; }` invocation. It has no metavariables,
parser fragments, interpolation, formatting macros, or assertion macros.
Its main function exits nonzero if the literal expansion and ordinary function
body produce the wrong value. The independent evidence sequence is:

1. Off reference in a new evidence cache.
2. Cold on in another new evidence cache.
3. Unchanged warm on in that same on cache, in a new compiler process.
4. An unrelated same-width body edit with warm on in the same cache.
5. Off after in the same cache.

All five executions retain exact status/diagnostic/runtime comparisons. They
use the same fixture source and output paths as each other, `verify-ich`, and
`-Zself-profile-events=default,function-args`. Each invocation has its own new
profile directory, retained under `native-profiles/`. Exactly one `.mm_profdata`
file must exist in each. The bundled `query_profile.py` parser validates the
raw capture and records its SHA-256, metadata, size, record count, and unresolved
record count. Full-profile virtual query IDs can be unresolved. They remain
unknown, never inferred, and are not used for native-hit proof.

The native marker schema consists of these exact resolved `GenericActivity`
interval labels:

- `jr_declarative_macro_eligible`
- `jr_declarative_macro_ineligible`
- `jr_declarative_macro_cache_lookup`
- `jr_declarative_macro_cache_provider`
- `jr_declarative_macro_cache_success`

The off runs must have zero such markers. Cold on must have positive and equal
eligible, lookup, provider, and success counts, with zero ineligible events.
Both warm runs must have positive equal eligible, lookup, and success counts,
zero ineligible events, and **strictly fewer provider executions than lookups**.
Only after the cold provider calibration may that difference be interpreted as
native query reuse. The report labels this difference `inferred_provider_avoided`,
not a dedicated hit-counter measurement. Zero eligible invocations, absent
markers, all-provider execution, a missing profile, or malformed profile data
fails `--require-native-hit` even if every semantic case passed.

The literal gate proves reuse only for the audited literal match/transcribe query, not
full declarative expansion caching or a speedup. A wider native implementation
or changed marker contract requires a reviewed harness schema update. The
matrix still tests all fallback semantics outside this narrow subset.

## Independent TT acceptance gate

`--require-tt-hit` is independently opt-in. It does not enable, replace, or
weaken `--require-native-hit`. Request both for candidate promotion. Without
the TT option, the original **32 revisions times seven modes**, plus the
optional five-process literal proof, remain unchanged. With it, 19 additional
TT revisions run all seven modes, for **51 revisions times seven modes**.
The added matrix covers identity and substitution, comma-separated and empty
repetitions, nested repetitions, repetition-count mismatch diagnostics and
recovery, definition/input/body changes, hygiene, span relocation, edition
transitions, invalid-input recovery, and `expr` AST interpolation forwarded
through a TT macro. The latter must preserve expression grouping, not flatten
an interpolated AST into ordinary tokens. Existing fragment and edition
fallback cases remain mandatory, including the parenthesized const-block
edition fixture.

The independent five-process TT proof uses only:

```rust
macro_rules! tokens { ($($t:tt)*) => { $($t)* }; }
fn body() -> i32 { 1 }
fn main() { if tokens!(40 + 2) + body() != 43 { std::process::exit(9); } }
```

There are no literal-only macro candidates, `expr` fragments, formatting
macros, assertion macros, or additional invocations that could lend unrelated
positive counters to this proof. The unrelated body edit changes `1` to `2`
and the checked total `43` to `44`. As in the literal proof, the five processes
are off-before, cold-on, unchanged warm-on, body-edit warm-on, and off-after.
The last four share `tt-proof/shared`, distinct from every semantic and literal
proof cache. Profiles live in `tt-profiles/`, separate from `native-profiles/`.
All runtime expectations and exact comparisons remain mandatory.

The **unchanged native marker schema** and strict positive calibration apply
independently: cold eligible/lookup/provider/success must be equal and positive,
ineligible zero. Each warm lookup must succeed, with positive lookup counts and
strictly fewer providers. Both explicit off controls must emit zero markers.
Absent TT support must fail, not skip or pass with reduced expectations.
A literal-only frozen compiler is the required native negative control:
even if its literal proof passes and all semantics agree, its absent/rejected
TT path must fail `tt_hit_verdict` and the overall verdict. Mock tests simulate
this exact split but do not establish that native negative control.

The report records `tt_hit_required`, `tt_hit_verdict`, and `tt_evidence`
separately from the literal evidence. A TT proof failure retains all five raw
observations and the passed semantic/literal verdicts. Root alone runs the
native negative control and new-frozen-compiler gates after compiler release.
Do not lower counts or accept all-provider execution to make TT absence pass.

## Capability gates and audit trail

Before fixtures, a minimal source probes both native flag values together with
verify-ich. Explicit unknown-unstable-option diagnostics produce verdict
`unsupported` and CLI exit `2`, not a passing gate. Other probe failures,
including crashes, fail with exit `1`. Neither case omits failed test cases and
reports success.

Tracing is the sole optional case. A real minimal macro expansion probes
`-Ztrace-macros=yes`. Successful probing must emit a `trace_macro` diagnostic.
Only explicit rejection of that option is recorded as an unsupported tracing
case. Any other tracing probe or traced fixture failure fails the gate. The
report distinguishes planned and completed cases and records the unsupported
tracing observation explicitly.

`acceptance.json` is written even after comparison failures. It includes the
full fixture source, expected output, commands, raw diagnostics, mode results,
capability probes, and completed-case count. Before and after execution it
records the exact compiler executable SHA-256, raw `-vV` output, sysroot query,
and resolved driver library paths and SHA-256 values. Any compiler or driver
change during the run invalidates the result. Preserve this report with native
acceptance evidence. A passed mock suite validates the harness contract only,
not the forthcoming compiler feature or its performance.

## Retained native TT evidence, October 30, 2026

`bench/results/declarative-tt-cache-20261030/native-evidence.tar.gz` retains
the complete positive `acceptance-tt/acceptance.json`, all ten native profiles
(five literal and five TT), the complete old-literal-only negative report and
log, positive CLI log, compiler build/version logs, before SHA-256 identities,
after identity verification log, and root's full Python test log (610 passed).
It intentionally excludes compiler binaries, executable fixtures, and build
caches. Original absolute paths remain in the raw evidence, not rewritten.

The candidate passed all **51 revisions times seven modes = 357 main compile
observations**, plus five literal and five TT proof processes. Dependency
compiles within the matrix are additional, not included in 357. Both native-hit
verdicts passed. Each proof calibrated one cold provider execution and showed
one inferred provider avoided in each unchanged and body-edit warm process.
Explicit off controls had zero markers. Frozen compiler executable SHA-256:
`570faeb27338cea8625c4a4a7eaeef71fc75470508657ff489c3da96ce5f9144`.
Driver SHA-256:
`ac116b9398e1f5c677e73952b8719be9d9e47190fe61fbaeba9b7cf270739595`.
Before and after identities matched.

The old literal-only compiler independently passed all 357 semantic modes and
the literal proof, then failed the TT proof with exactly
`Cold tt fixture did not positively exercise native eligible/provider/success paths`.
Its cold TT counters showed positive ineligible events and zero eligible,
lookup, provider, and success events. This establishes the native negative
control, not just a mocked absence test.

Replay the portable audit without invoking rustc or fixture binaries:

```sh
python3 bench/results/declarative-tt-cache-20261030/audit.py
```

The audit repeats every exact status/stdout/stderr comparison and independent
runtime/error expectation for both 357-mode matrices, checks sources, mode
flags and shared-cache paths, verifies unchanged identities, validates the two
positive proofs and exact negative failure, and reparses all ten retained raw
profiles to verify hashes, sizes, metadata, resolved marker counts, and unknown
record counts. `audit.json` records the successful replay and archive hash.
The audit does not require any original scratch paths to exist. These results
prove narrow TT reuse and correctness for the covered cases, not a speedup,
general TT transcriber eligibility, or default promotion.
