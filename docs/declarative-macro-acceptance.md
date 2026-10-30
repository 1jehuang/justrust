# Native declarative macro cache acceptance

`bench/reuse-declarative-macro-accept.py` is the correctness gate for the
forthcoming default-off `-Zjr-cache-declarative-macros=yes/no`. It follows the
native dead-code retention transition harness, but exercises declarative macro
expansion. It is not a speed benchmark or evidence of a cache hit.

## Execution boundary

Only mocked Python unit tests have been run during harness implementation.
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
  --output /absolute/path/to/new-declarative-macro-acceptance
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
The harness does not infer eligibility or promise a hit counter contract.

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
