# Real-program native compiler checkpoint, October 18, 2026

## What was actually compiled

Two standalone algorithm programs use **identical source bytes** for `jr-rust`
and rustc. No fallback, compiler-specific source branch, or substituted host
algorithm is used on the native path.

- `experiments/programs/mandelbrot.rs`: a newly authored 120 by 40 ASCII image
  renderer, up to 200 floating-point iterations per pixel. This is a real image
  computation, but deliberately written within the supported subset, not an
  unchanged external project. Fixed dimensions and literal pixel output avoid
  unsupported argument parsing and general formatting.
- `fixtures/adler2_adapted/cases/ff_5553.rs`: an existing serial Adler-32 program
  processing 5,553 bytes. It is an **adaptation**, not unchanged Adler2 library
  compilation. Its original provenance, license and algorithm changes are in
  `fixtures/adler2_adapted/README.md`. Stock and native both compile the exact
  same adapted program. No upstream four-lane throughput comparison is implied.

Every executable must exit zero and produce its entire expected stdout.
The renderer is checked against an independent Python calculation, and the
checksum against Python's zlib (`2385091723`). The complete native image is
retained verbatim in the renderer receipts' stdout fields, SHA256
`e198888476e776062ec4198c9529af9a22f3b497467348d1b422e910531fa47b`.

## Compiler and measurement identity

Native is a release-built **committed snapshot** of
`af3eb295fae9`, SHA256
`59eaaa6bb98c405471bac949f5a6dbc76fb0aea8ac919f0c724c0b4c5ed23d63`.
It was built separately from a `git archive` of Cargo files and `src/` so ongoing
uncommitted character-cast implementation work cannot affect this benchmark.
The stable justrust client was not replaced. Bootstrap uses installed justrust,
with local fallback reporting that the remote machine was not running.
Stock is `/usr/bin/rustc` 1.98.1, commit `48a229cea`, LLVM 22.1.8.
Both compilers and all measured programs execute locally on the same Intel
Core Ultra X9 388H Linux laptop.

An initial unrestricted-affinity run had substantial variation. It is retained,
not discarded. The follow-up pins the entire runner and child processes to CPU 3,
a performance core, with native workers=1. This also constrains rustc's threads:
it is a controlled single-core comparison, not a maximum-throughput comparison.
Frequency, temperature and unrelated processes are not controlled.

One untimed build/run per configuration precedes 21 measured builds and 31
measured executable runs. Configuration order rotates. All build outputs are
fresh files, all invocations fresh processes, and filesystem pages are warm.
Timings include process startup and output capture. These are **not** cold-cache,
incremental, no-change, Cargo dependency, or hosted compilation measurements.

Stock has two settings: opt-level=0 and opt-level=3. Both retain overflow checks,
debug assertions and panic=abort, edition 2024. Native retains its default checked
arithmetic and debug assertions. It has no optimizer equivalent to rustc O3.
Stock links its normal runtime; native emits its private direct ELF image.
Mandelbrot runtime includes per-pixel printing with different runtime buffering,
so this is end-to-end program latency, not isolated floating-point throughput.

## Controlled-core results

Median process wall milliseconds, with observed min/max in parentheses.
All 228 recorded events in each follow-up receipt succeeded, including metadata,
compilation and complete-output validation. No failed run is timed as a success.

| Program | Compiler | Build, 21 samples | Run, 31 samples |
|---|---|---:|---:|
| Mandelbrot | Native | 6.383 (6.299–7.334) | 5.389 (5.216–5.718) |
| Mandelbrot | rustc O0 | 36.978 (36.190–37.627) | 1.655 (1.615–1.734) |
| Mandelbrot | rustc O3 | 46.807 (46.286–48.322) | 1.011 (0.976–1.039) |
| Adapted Adler-32 | Native | 8.208 (8.122–10.211) | 0.250 (0.242–0.307) |
| Adapted Adler-32 | rustc O0 | 36.833 (34.961–37.460) | 0.378 (0.353–0.429) |
| Adapted Adler-32 | rustc O3 | 45.212 (45.048–47.297) | 0.365 (0.343–0.414) |

The useful signal is low native compile latency on these two tiny programs,
with substantially slower native Mandelbrot execution. The sub-millisecond
checksum timings are strongly affected by startup/runtime overhead and do not
establish superior checksum throughput. These are scoped observations, not a
general compiler speedup, full Rust claim, or replacement for the broad suite.
The historical `bench/suite.py` and `bench/versus.py` target the previous Cargo
service, not this native source compiler. No default promotion is proposed.

Raw receipts under `experiments/results/`:

- `20261018-real-mandelbrot.json`, `20261018-real-adler2-adapted.json`:
  initial unrestricted runs, 9 build and 15 runtime samples per configuration.
- `20261018-real-mandelbrot-pcore.json`,
  `20261018-real-adler2-adapted-pcore.json`: the complete controlled-core samples
  summarized above, with binary/source hashes, actual commands, outputs and
  toolchain identity. The follow-up runner adds affinity metadata; its different
  runner hash is intentional. No algorithm or timing logic changed.

## Validation and reproduction

The Python harness has 16 passing tests, including exclusive output creation,
timeouts, cleanup, measured failure exclusion and runtime oracle mismatches.
Synthetic fake-compiler tests validate the harness only. The actual compiler
and executable runs above provide end-to-end acceptance evidence.
Formatting passes (`cargo fmt --check` and standalone `rustfmt --check`).
Hosted committed-snapshot CHECK, strict Clippy and all three CLI tests pass.
An initial hosted check failed because an absolute manifest path was outside
its mirrored workspace. Rerunning from the snapshot root fixed the invocation.
The failure and successes, plus bootstrap, are exported in
`data/runs/xps14/2026-10-18-real-program-benchmark.jsonl`:

- bootstrap: `20261018-015714350-320588`
- failed hosted invocation: `20261018-015802650-322591`
- hosted check: `20261018-015859161-324456`
- hosted strict lint: `20261018-015906849-324454`
- hosted CLI tests: `20261018-015955324-327462`

With a release-built compiler from the same snapshot, fresh output paths and a
suitable performance core for the machine:

```sh
taskset -c 3 python3 experiments/real_program_bench.py \
  --binary /path/to/jr-rust --compiler-revision af3eb295fae9 \
  --repeats 21 --runtime-repeats 31 --output /path/to/new-mandelbrot.json

taskset -c 3 python3 experiments/real_program_bench.py \
  fixtures/adler2_adapted/cases/ff_5553.rs --oracle adler2-ff5553 \
  --binary /path/to/jr-rust --compiler-revision af3eb295fae9 \
  --repeats 21 --runtime-repeats 31 --output /path/to/new-adler32.json

python3 -m unittest discover -s experiments -p test_real_program_bench.py -v
```

The remaining real-project gate is still unchanged external source, native
core/alloc/std, project manifests/dependencies and realistic larger workloads.
Neither of these intentionally bounded standalone programs closes that gate.
