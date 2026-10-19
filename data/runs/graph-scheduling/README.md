# Early root scheduling and canonical checking validation

These are selected, path-redacted `justrust export` records for the graph scheduling lane. They are correctness evidence, not compiler throughput benchmarks.

- `23ab0b3de` requests closed nongeneric analysis/object tasks before awaiting `MONO_PLAN`. The instance plan still exists; this does not complete graph decomposition.
- `94482d101` repairs the direct semantic checker to retain canonical Source lowering proofs. It also replaces one obsolete no-object-task assertion with no object publication, no executable publication, and no object demand during check-only operation.
- `20261018-203116335-2620496`: 56 semantic integration tests passed across parameter equality, associated equality/bounds, generic carriers, modules, external modules, and tuples.
- `20261018-203216427-2643328`: 1,377 library tests passed.
- `20261018-203226723-2643147`: strict all-targets Clippy passed.

These runs do not establish a full-suite pass for the concurrent ProofSession, compact reservation, and generic Drop integration. The depth-129 and 1,165-function resource regression gates remain unchanged and require composed revalidation. No speedup claim follows from these records.
