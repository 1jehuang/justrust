# Sparse ownership accounting recovery

Validation used the installed stable justrust client on hosted, with no experimental install. Final source base was `427b5db32`, plus only the ownership accounting and graph size ratchet changes. The six selected records are redacted exports, not benchmark comparisons.

- `20261018-194443850-1153792`: temporary binary probe on the preceding base found 996 generated functions compile and 997 fail. The temporary binary-search code was removed.
- `20261018-194603349-1204567`: final graph integrity acceptance, 5 passed. Reconfirmed 996 compiles and 997 fails with `Work`, requested 1994, used 31999051, limit 32000000, `src/compiler/ir.rs:1941:47`. Planning work growth was 1.692 and leaf closure growth 1.000. Existing growth ceilings were not loosened. Additional fixed native-heap source proofs change the work ratio, not the algorithm.
- `20261018-194608807-1210164`: hosted check passed.
- `20261018-194616861-1215477`: 140 ownership unit tests passed, including the new exact capacity accounting and checked overflow regression.
- `20261018-194630669-1222976`: 29 ownership and accounting integration tests passed.
- `20261018-194645086-1210163`: strict all-target clippy with `-D warnings` passed.

The orphan change removed a stale dense per-path selection charge. Sparse entries contain two usize values, not one. Merely charging entry count would undercount Vec geometric capacity, including the minimum allocation for one entry. `Selected::set` now reserves exactly before insertion. The regression measures retained capacity after clearing, covers zero and non-power-of-two slot counts, and checks arithmetic overflow. Preflight uses variant-offset count as a conservative upper bound, runtime uses actual enum-path count.

This is a resource-accounting correction and a stronger checkpoint, not completed scalability or Rust compatibility. The unchanged fixed 32-million work limit still fails at 997 functions. No limits or debt baselines were raised, and no compile-time speed claim is made. Main was neither edited nor built by this lane. Original WIP and subsequent recovery checkpoints remain in the timestamped external backup directory and retained stashes.
