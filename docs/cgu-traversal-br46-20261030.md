# Shared per-CGU inline traversal br46, 2026-10-30

**Non-win. No general performance or default-promotion claim.** The protected br46 timing session completed before this offline retention task. Its candidate median is higher than the baseline median. No defaults change, and the native option remains off by default.

| Stack | Median edit-to-filtered-test wall time, seconds |
| --- | ---: |
| Owned, earlier experiments off | 1.137701 |
| Owned, earlier three experiments on | 1.140851 |
| Owned shared per-CGU inline traversal candidate | 1.233052 |
| Blaze, pinned Cranelift backend | 0.093685 |
| Blaze, pinned LLVM backend | 0.102377 |

These are descriptive medians from five edits per stack in one seeded-warm, four-filtered-test session. They do not establish statistical significance, a general regression magnitude, or broader Rust compatibility. The owned-versus-Blaze focused-loop gap remains. No separate profile run, including dp47, is a speed claim or part of these timings.

## Native candidate and identity

The native implementation is recorded in commit `66e0f9273fd079a7a2334d8b89aafeeade9e8c76`, with the tracked unstable option `-Zjr-cgu-shared-inline-traversal=yes`. Its implementation shares inline-successor traversal state within a codegen unit rather than repeating the complete walk for each root. Roots are still expanded, and visited inline items remain scoped to their codegen unit. See the separately maintained native implementation document for architecture and correctness work. This retention task did not execute or change that implementation.

The recorded frozen compiler driver SHA-256 is **`29ea709935d751ce13e77995173659006c1e15683e0f0645616d12cd5a3a1665`**, identical before and after. All three owned stacks use that same compiler driver, so the comparison changes the candidate flag rather than swapping compiler identities between stacks. The saved untimed `rustc -vV` receipt reports `rustc 1.101.0-nightly (38a07e7de 2026-10-14)`. The executable hash alone is not sufficient compiler provenance, so the validator independently pins the recorded driver hash as well as comparing the complete before/after identity maps. Replay validates retained receipts, not present-day executable contents or instruction-level branch execution.

The candidate sets the earlier three reuse experiments to `no`, adding exactly `-Zjr-cgu-shared-inline-traversal=yes`. `owned-off` has only the three earlier flags set to `no`, and `all3on` sets those three to `yes`, with no candidate option. All owned stacks use the same official Cargo path in `CARGO`, `JUSTRUST_REAL_CARGO` and recorder `real_cargo`. Proc-macro caching, buildd, dependency/local/peer/remote/prebuilt caches and toolchain auto-installation are disabled. No native Cargo daemon is enabled. HOME, target and all XDG roots are stack-private. Fixed dev/test profiles use opt-level 0, debug 0 and incremental true. Blaze's backend is explicitly `cranelift` or `llvm`, both at `BLAZE_OPT=0`.

## Runtime gates and retained artifacts

All five stacks passed five untimed gates: prepare, deliberate E0308 rejection with `JR_RUNTIME_TYPE_ERROR`, type recovery, deliberate test failure with `JR_RUNTIME_FAILURE`, and test recovery. Each then passed five timed edits. There are **50 rows, 25 accepted timed edits, and no exclusions**. Every success has exactly the four expected `summary::tests::` names, four passes, no failures and 144 filtered out. The intentional failure affects only `shares_overlapping_time`. Each timed edit prints precisely its own `JR_RUNTIME_EXECUTED_1` through `_5` marker once. All five stacks receive identical edit bytes for a replicate, and stack order rotates across the five replicates.

Before/after source maps match and the original edited source is restored. Injected source hashes are independently reconstructed from `source.original`. Tool identity maps and Blaze seed source maps agree before/after. Both Blaze stacks receive the same recorded br30 seed content. The scope is **warm-store seeded**, not cold support. CPU/noise records explicitly classify contamination as unknown, not an established quiet machine.

`bench/results/cgu-traversal-br46-20261030/` retains the exact completed root JSON, original source, flat stdout/stderr, all 30 owned raw-output journals, historical launcher/launch logs/exit receipt, compiler-version receipt, and all thirty dedicated private recorder directories including their raw journals, metadata, summaries and process/sample state. No targets, caches, executables or sockets are included. Absolute paths inside artifacts are historical provenance, not portable replay inputs. Raw journals preserve blank lines and are the owned runtime authority, not condensed console output or slim exports.

Thirty calls were exported exclusively from the three private br46 recording homes, ten per home, into bundle-local `exports/`:

- `xps14-runtime-br46-owned-off-20261030/2026-10.jsonl`
- `xps14-runtime-br46-owned-on-20261030/2026-10.jsonl`, corresponding to `all3on`
- `xps14-runtime-br46-owned-cgu-traversal-20261030/2026-10.jsonl`

No global export or existing `data/runs` directory was used or modified. The validator matches all thirty export IDs and exit statuses to their exact recorder states and row provenance.

## Portable offline audit

```sh
python3 bench/results/cgu-traversal-br46-20261030/validate.py
python3 bench/results/cgu-traversal-br46-20261030/test_validate.py
```

The standard-library-only validator uses only the bundle, even when copied out of the repository. It invokes no compiler, Cargo, client, Blaze or daemon. It checks the full file inventory and SHA-256 manifest including validator/test sources, all fifty unique row keys and gate ordering, rotated 25-edit order, original/injected source hashes, exact raw-journal provenance and paired recorder bytes, independently parsed tests and unique runtime markers, negative/recovery gates, exact flags and private isolation roots, fixed profiles, complete source/tool/seed receipt equality, pinned compiler driver, version receipt, thirty recorder metadata/status records, thirty dedicated export IDs/statuses, and recomputed medians.

The mutation suite includes copied-directory golden replay, missing/extra/tampered artifact rejection, baseline candidate-flag leakage, disabled/wrong candidate flags, wrong Cargo/driver/version/source/seed identities, timeouts, isolated-home leakage, forged medians and recorder/export identity/status changes. Raw mutations update both retained journal copies and their row hashes, so stale/duplicate runtime markers, missing E0308/error/failure markers and broken type/test recovery must fail semantic gates, not merely integrity checks. Use normal Python, not `python -O`, for assertion checks.

`seal.py` creates the one-shot inventory and refuses to overwrite it. `retain.py` and `launch-runtime46.sh` are historical utilities, not portable replay commands. No native workloads were run for this retention task. Only this new result directory and this new document were changed.

## Limits

This diagnostic is neither a full-suite acceptance result nor a comprehensive session-weighted before/after benchmark. No check/build/cold-start conclusion or default promotion follows. Native implementation correctness and separate profile evidence remain distinct. Do not compare different sessions as controlled before/after samples, and do not turn this protected timing non-win into a profile-derived speed claim.
