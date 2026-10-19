# Early root scheduling and canonical checking validation

These are selected, path-redacted `justrust export` records for the graph scheduling lane. They are correctness evidence, not compiler throughput benchmarks.

- `23ab0b3de` requests closed nongeneric analysis/object tasks before awaiting `MONO_PLAN`. The instance plan still exists; this does not complete graph decomposition.
- `94482d101` repairs the direct semantic checker to retain canonical Source lowering proofs. It also replaces one obsolete no-object-task assertion with no object publication, no executable publication, and no object demand during check-only operation.
- `20261018-203116335-2620496`: 56 semantic integration tests passed across parameter equality, associated equality/bounds, generic carriers, modules, external modules, and tuples.
- `20261018-203216427-2643328`: 1,377 library tests passed.
- `20261018-203226723-2643147`: strict all-targets Clippy passed.

These runs do not establish a full-suite pass for the concurrent ProofSession, compact reservation, and generic Drop integration. The depth-129 and 1,165-function resource regression gates remain unchanged and require composed revalidation. No speedup claim follows from these records.

## Composed capture and Debug checkpoint `40feb23c8`

- `20261018-204045518-2788311`: hosted all-targets check passed.
- `20261018-204100081-2789922`: compiler and graph tests reported 30 passed and one failed. All seven graph gates passed, including the unchanged 1,165-function gate. The original growing-type profile control still exhausted work at 32,134,173 in `MONO_PLAN` before its expected depth-129 rejection. This checkpoint does not contain the pending TypeWalk growth recovery.
- `20261018-204121969-2792890`: both full-program corpus tests passed their existing ratchet. Actual native compatibility remains **3/20**, not 20/20: hello, Collatz, and GCD/Fibonacci pass unchanged.

The corpus first diagnostics at this checkpoint are unresolved types (02, 07), unknown function-local macros (04, 13), const items (06), derive attributes (08, 12, 14, 15, 18, 19, 20), or-patterns (09), stored-closure syntax (10), generic closing syntax (11), range indexing (16), and non-local-rooted assignment (17). These are observed first blockers, not claims that implementing each alone completes its program. Sources and manifest expectations were not modified.
