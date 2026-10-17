# Shared-slice rejection public CLI gate

`slice-contract-rejection.py` is an executable acceptance runner, not a compiler
implementation. Default execution is **stock-only**. Native slice integration
has not been validated. The runner never builds/installs a compiler and never
substitutes rustc for a native command.

## Commands

From the repository root, with new output directories:

```sh
./experiments/slice-contract-rejection.py --self-test
./experiments/slice-contract-rejection.py \
  --output-dir /home/jeremy/.jcode/scratch/slice-rejections-stock-new
# Explicit post-integration gate, expected to FAIL on pre-slice compilers:
./experiments/slice-contract-rejection.py \
  --native /absolute/path/to/jr-rust \
  --output-dir /home/jeremy/.jcode/scratch/slice-rejections-native-new
```

Optional `--fixtures DIRECTORY`, `--rustc EXECUTABLE`,
`--positive-control SOURCE` and `--timeout-seconds N` select inputs. All fixtures
must be standalone edition-2021 files, without external dependencies. Each has
exactly one `// stock: accept` or `// stock: reject` and one
`// category: NAME` header. `excluded` expects native `unsupported` diagnostics,
independently of stock validity. Other categories require semantic rejection,
not `unsupported`, parser errors or a compiler panic. At least `lifetime`,
`unused-invalid-body`, and `excluded` categories are required. A stock-accepted
negative must be explicitly classified `excluded`.

The independent fixture owner supplies
`tests/fixtures/slice-contract-rejection/*.rs`. The initial corpus has six
Rust-invalid programs and six legal-but-excluded programs. Native admission
classification is separate: bare by-value slices are excluded even though
stock rejects them. Lifetimes are tested
inside the admitted parameter/local domain, not through excluded slice returns.

## Native assertions

A native run first compiles and runs the stock positive control, defaulting to
`fixtures/adler2_adapted/cases/four.rs`. The same source must pass native
`--check` and native build, both with `--workers 1` and `--workers 8`.
Build outputs must be ELF files and match stock exit/stdout/stderr. This positive
admission gate prevents a compiler that rejects all slices from passing the
negative suite. A positive program must contain shared byte-slice syntax.

Each rejection then runs all four native combinations:

```text
jr-rust SOURCE --workers {1,8} --profile NEW_JSON --trace NEW_TRACE --check
jr-rust SOURCE --workers {1,8} --profile NEW_JSON --trace NEW_TRACE -o NEW_OUTPUT
```

The check command deliberately omits `-o`: the public CLI forbids their
combination. Every command runs in a fresh private directory with an exact copy
of the source. Nonzero exit, nonempty appropriate stderr, a failure trace and
schema-1 failure profile are required. Graph capture must be complete, with
zero dropped events and zero omitted nodes. Both full node summaries and phase
summaries must omit object operation kind 103, including zero-duration nodes.
Graph failure count must be positive. Failed builds and checks cannot publish
an executable or any unexpected directory entry. Successful check controls
also forbid object work. Successful controls must show ownership analysis
(kind 109 or 112), with object work required in successful builds.

These checks use `src/compiler_cli.rs:15,79-80,95-124,129-143` for flags,
check/output exclusivity, failure profile/trace emission and delayed executable
publication. `src/compiler_profile.rs:32-45,100-116,158-194` defines schema,
mode/workers/success, phase IDs, capture counters, omitted-node count and node
records. Existing rejection precedent is
`tests/generic_carriers_graph.rs:117-151`.

## Evidence and limitations

A new `--output-dir` is mandatory and is never reused. `evidence.json` retains
source/runner/binary SHA-256 identities, `rustc -Vv`, exact arguments, exits,
full stdout/stderr and pass/fail status. Sources and native profiles/traces are
copied into the evidence directory, including a failed positive admission
attempt. Temporary native executables are removed after the run. Rustc may be
a rustup proxy: its hash records the invoked file, while `-Vv` identifies the
selected toolchain. Native executable identity is hashed directly.

The gate is fail-fast, so a failed positive control means no native negatives
were exercised. Full graph capture and no omitted nodes are mandatory even if
a later compiler grows beyond the current profile limit. This intentionally
fails rather than silently weakening a nonpublication claim. Diagnostic checks
are coarse category assertions, not proof that the compiler found the precise
intended error. The runner does not establish per-function edge ordering,
universal ownership proof correctness, global filesystem sandboxing, emitted
ABI correctness, no-rustc-exec syscall tracing, runtime bounds behavior,
unchanged Adler2 crate support, or performance improvement. Those need separate
integration and semantic acceptance gates.

## Validation performed before slice integration

- `--self-test`: accepted a valid failure graph and rejected eight corrupted
  graph variants plus four invalid diagnostic variants.
- All 12 independent fixtures passed stock metadata expectations with rustc
  1.98.1, commit `48a229ceaefd4985c50990b14116b6d856af0985` (see actual `-Vv`
  in evidence for authoritative identity).
- Eight actual baseline CLI probes used a plain nonslice positive and a plain
  type error, across workers 1/8 and check/build. Graph schema, analysis/object
  assertions and failed-output nonpublication passed. This validates runner
  plumbing, **not shared-slice support**.
- Native execution using the existing `target/debug/jr-rust` failed the Adler
  positive-control check as expected. No integrated native success is claimed.

Scratch evidence is under `/home/jeremy/.jcode/scratch/`:
`slice-harness-real-schema/`, `slice-rejection-stock-panda/`, and
`slice-rejection-preintegration-panda/`. Scratch evidence is not a committed
benchmark or permanent CI artifact. Run commands above to regenerate evidence.
