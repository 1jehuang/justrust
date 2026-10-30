# Compiled-test comparison observations, 2026-10-30

## Final accepted warm workflow

`br35` additionally passes all six stacks, including explicit Blaze LLVM with
the same seed. Run `validate-complete.py --case br35`. Heavy contention and
the slower hotpatch median are retained, not filtered out. It is a stronger
backend-family acceptance check, not a controlled speed estimate.

`br34` now contains a complete accepted five-stack seeded warm-store comparison.
Run `validate-complete.py` for its raw-log, matched-edit and export audit.
See `docs/blaze-runtime-results-20261030.md` for interpretation and limits.
`br33` preserves the failed hotpatch-routing setup preceding that final run.
The historical audit below and `validate.py` apply only to `br30` through `br32`.

## Historical first three attempts

These are retained observations, **not a complete accepted Blaze versus owned
runtime speed comparison**. Do not combine accepted stacks from different
attempts into a headline ratio. Source pin is `f97fcc2`; the exact four-test
contract is in `docs/blaze-runtime-contract-20261030.md`.

- `br30`: Blaze passed all deliberate type-error/test-failure/recovery gates and
  five unique executed edits. Owned builds ran the same four tests successfully,
  but were excluded because the first harness could not recover raw passing test
  output from justrust's condensed UI. Its missing parser result is not a build
  failure. The later harness retrieves a uniquely fresh persisted raw journal.
- `br31`: both owned modes passed the full contract and five edits. Blaze failed
  initial preparation on unsupported writable statics, so it has no timing rank.
- `br32`: both owned modes again passed. Explicitly pinning Cranelift and opt=0
  did not resolve Blaze's preparation failure. All archived source hashes and
  tool identities match br30. This contradicts an earlier LLVM-only explanation.
- `probe-cranelift` succeeded and `probe-llvm` failed in the initial compatibility
  probes. These are setup observations, not measured warm-edit comparisons.

Each attempt retains stdout/stderr, source manifests, exact executable/driver
identities and raw owned journals when available. Metadata retains noise proxies
as unknown contamination. Setup and correctness gates are not timed edit samples.
No quiet-machine, significance, comprehensive-workload or default-promotion claim
is supported by these observations.

## Architectural evidence

The successful br30 Blaze worker log reports one function compiled and 44,562
reused on later edits, with seven patch functions and six redirected functions.
Reported spliced-link scopes are approximately 1.7-2.0 ms. These are Blaze's own
scoped counters, not independent CPU attribution or a demonstrated causal split
of the end-to-end gap. They do establish that the mechanism is more than simply
putting the same compiler's disk cache into RAM: function-granular compilation
and executable patching are part of the observed path. justrust already contains
an opt-in hot-patch implementation in `src/hotpatch.rs`; its value must be measured
under the same executed-output gates rather than assumed.

## Native fine-grained analysis experiment

`native-dead-code/acceptance.json` retains all 32 revisions x seven modes = 224
compiler invocations for the default-off `jr-cache-dead-code-items` experiment,
implemented in commit `fdb2fbc96`. It keeps global reachability unchanged while
caching each item's scanned usage edges. Exact diagnostics/status/runtime and
incremental stable-hash verification passed. This correctness gate does not prove
real-project cache hits, wall-time savings, or a global reuse ceiling. Those are
separate acceptance questions. The compiler driver SHA is
`3527393f3db7e09eeef270a83f51c5aeedb787c77f845ab76a76618f76ae137e`.
