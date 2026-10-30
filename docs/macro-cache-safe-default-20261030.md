# Procedural macro cache safe default, 2026-10-30

The shipping recorder no longer automatically injects `-Zcache-proc-macros` on owned incremental toolchains. It now requires explicit `JUSTRUST_CACHE_PROC_MACROS=1/on/true/yes`. Missing, empty, unknown and negative values leave it disabled. Native rustc's flag already defaults to false. An explicitly supplied compiler flag remains an expert experiment, not a correctness-certified optimization.

## Why

The actual-family differential gate reproduces stale generated values in both derive and attribute macro caches after external environment/file changes, including tracked environment/path APIs. Function-like macros were only an uncached control in the first harness, so their passing results could not justify enabling these different cached families. See `docs/macro-cache-input-contract.md` and its retained native report for exact compiler identities and observations. The instrumented macro-cache-on latency diagnostic is not an eligible safe optimization.

This change is a correctness fix, not a speed improvement. It may remove an unsafe warm-build speedup. Native dependency tracking and side effects must be repaired or rigorously constrained before reconsidering automatic enablement. Declarative macro caching is a separate default-off experiment with its own semantic gate.

## Validation

Hosted validation remained paused under the earlier maintenance request, so these correctness checks ran explicitly locally. No hosted performance workload was substituted or claimed. The unrelated active clean-room owner was notified before local validation.

- `cargo fmt` and warning-free `justrust clippy --local -- -D warnings`.
- `justrust check --local`: success, run `20261030-042512101-1859909`.
- Four focused shim tests: success, run `20261030-042520904-1861102`. Includes unset/empty/negative/unknown policy, all four explicit positive values, non-owned compiler, non-incremental unit and duplicate flag checks.
- `justrust test --local`: 847 passed, zero failed, 12 ignored, run `20261030-042612239-1866782`.
- Clippy run `20261030-042549339-1864478`.
- `cargo install --path . --locked`: installed corrected shipping client.
- Actual installed-client recorded/unrecorded/recorded compatibility fixture: all successful, unrecorded Cargo reports Fresh without Dirty or a compiler invocation, and recursive rmeta hashes are identical. Recorded runs `20261030-042837633-1891779` and `20261030-042837808-1891834`. Direct Cargo is intentional here to test the unrecorded side of the recorder boundary.
- Complete benchmark Python suite before later harness additions: 580 tests passed.

Installed client identity, exact fixture logs, runner and artifact hashes are retained in `bench/results/macro-cache-safe-default-20261030/`. This is a narrow recorder compatibility and safety-policy check, not a benchmark or proof that explicit unsafe proc-macro caching is now safe. All cited runs are exported in `data/runs/`.
