# Generic pointer-owner Drop integration

The validated source snapshot is `a4776b447`, incorporating generic pointer-field owner Drop `885ccc768` through merge `7f9c6959e`. Full Rust and generic Box support remain incomplete. The full-program corpus remains 3/20.

## Observations

- Composed hosted check, 1,386 library/graph tests, and strict all-target clippy passed. Local stock-dependent scoped suites passed 43 tests.
- Initial full run `20261018-211007811-3501750` reported 3,405 passed and four failures, including child rerun counts. The two actual failing panic tests expected the old phrase `nongeneric scalar-field struct`. Both frozen unit-struct Drop controls remained Unsupported.
- Commit `a4776b447` changes only those two expected diagnostic strings. Source bytes, fingerprints, classifications, partitions, and abort/unwind observations are unchanged. Targeted panic plus graph gates passed 12/12 in `20261018-211545968-3727317`.
- Final full local acceptance `20261018-211613375-3734427`: **3,407 passed, zero failed**, 188 binaries, `JUSTRUST_TEST_JOBS=4`, `--no-fail-fast`. Formatting passed before the run. No isolated retry warning appeared.
- Strict all-target clippy `20261018-212248845-3983495` passed with zero warnings. A forced hosted attempt could not apply a synchronized file and fell back locally. This is local lint evidence, not hosted success. Hosted disk exhaustion was independently observed during this window.

## Publication and measurement boundaries

A concurrent benchmark-documentation push `9d6439ed9` also published its existing parent merge `7f9c6959e` before composed full acceptance finished. This ancestry race was identified and retained without force/reset. The passing final run above validates the repaired snapshot afterward. Subsequent publication is coordinated serially.

A separate generic timing experiment overlapped the initial full run and is not clean timing evidence. No speedup claim follows from these checks.

## Export format

These are selected owned, path-redacted `justrust export` records. Per-binary timing lists are replaced by `tests.binary_count`; aggregate verdicts, diagnostics, run ids, resources, and routing evidence are retained. Original run logs remain accessible through the stable client. Earlier passing baseline and original infrastructure/test-namespace failures remain in `../graph-final-acceptance/`.
