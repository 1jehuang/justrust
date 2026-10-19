# Graph recovery and final acceptance receipts

These selected, path-redacted `justrust export` records continue `../graph-scheduling/`. They are correctness evidence, not throughput benchmarks or a claim of full Rust compatibility.

## Changes under test

- `6c4f546f4`: paid geometric growth for the task-local TypeWalk memo. Existing proof caches and ProofSession behavior are unchanged. Both original resource contracts, depth 129 and the 1,165-function graph gate, remain unchanged.
- `fec3a375b`: promote the exact stock-accepted dyn Drop-owner fixture from boundary to positive. Source and expected stdout are unchanged. A fresh stock compile and native workers 1/8 executions all exit successfully with exactly `ok\ndrop\n` and no stderr. The regression checks every user body and its concrete proof, check-only absence of codegen, and object dependencies on proof output zero.
- `e239d4beb`: correct that regression's consumer identity mapping. Dependency edges identify consumer **tasks**, not nodes. The test now resolves the object-owning task through captured events instead of assuming the two ID namespaces coincide.

## Failed runs remain evidence

- `20261018-204339398-2803346`: original full local run, reported 3,393 passed and one failed after the runner's isolated retry. The initial run also had a stock linker SIGSEGV in a debug-assertion oracle. Its automatic isolated retry passed, but this is not a pristine full pass. The remaining failure was the now-supported dyn Drop fixture's obsolete boundary classification.
- `20261018-205025399-3040912`: separate, explicitly serial debug-assertion run: all four tests pass. Command: `JUSTRUST_TEST_PARALLEL=0 justrust test --local --test debug_assertions -- --test-threads=1`. This does not erase the original linker failure or establish its cause.
- `20261018-205313725-3061872`: full local run with `JUSTRUST_TEST_JOBS=4`, 3,394 passed and one failed. The new graph assertion incorrectly equated a consumer task ID with an object node ID. No compiler semantics were changed to repair it.

## Successful controls and final run

- `20261018-204253594-2799803`: paid-growth allocation/overflow regression passes.
- `20261018-204311595-2799506`: compiler plus graph gates, 31/31 pass, including the unchanged growing-depth and size contracts.
- `20261018-205227325-3057585`: trait-object and graph tests, 10/10 pass before the task-identity regression was exposed by the full run.
- `20261018-205814848-3244012`: corrected trait-object and graph tests, 10/10 pass. Twelve additional runs from `20261018-205821512-3244339` through `20261018-205825226-3245407` exercise the corrected graph test repeatedly at workers 1/8 in check/build modes; all pass. Every receipt is included.
- **`20261018-205830472-3245901`: final full local acceptance at `e239d4beb`, 3,395 passed, zero failed, zero ignored.** Command: `JUSTRUST_TEST_JOBS=4 justrust test --local --no-fail-fast`. No isolated-retry warning or initial failure appears in this run. Local execution is required for tests invoking `/usr/bin/rustc`; binary concurrency four is an explicit control, not a performance claim.
- `20261018-210245976-3245784`: forced hosted strict all-targets Clippy passes. `cargo fmt --check` also passes.

After that full run, `f8888ab66` merges remote-only documentation, recorded discovery results, and the new `source_error_order` test. It changes no compiler source. `20261018-210348282-3437746` then passes all 15 source-order and graph tests on hosted; `20261018-210354837-3437668` passes forced hosted strict Clippy. Formatting remains clean. The 3,395-test full-suite claim is specifically for `e239d4beb`, not a fabricated full run including the later eight tests.

The final full run contains the capture/Debug/canonical-checker composition and these recovery commits. It does not contain the separate pending immutable-cache, per-file parse, const, or macro feature work. The full-program corpus remains 3/20 as recorded in `../graph-scheduling/`; passing its ratchet is not full language support.
