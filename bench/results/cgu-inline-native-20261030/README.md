# CGU shared-inline native acceptance, October 30, 2026

Correctness and positive native mechanism evidence only. No speedup or default promotion.

## Retained artifacts

- `native-evidence.tar.gz`: unmodified original full failed report and four raw profiles, focused repaired report and four profiles, repaired full passed report and four profiles, original failure and repaired CLI logs/statuses, compiler build/status, frozen identity/version logs, source-ready marker, and root's Python test log.
- `harness-original.tar.gz`: harness at `719c916df` plus its complete local Python helper/parser sources.
- `harness-repaired.tar.gz`: harness at `20e3f2a5e` plus the same complete helper/parser sources.
- `native-source.tar.gz`: native compiler files and documentation at accepted source commit `66e0f9273`.
- `audit.py`, `audit.json`: detached offline audit and successful summary with all archive hashes.
- `test_audit.py`: ten mutation and genuinely detached replay checks.

The source-ready marker retains the original frozen partitioning/options hashes.
The auditor verifies both archived compiler files exactly against those hashes.
The native source documentation at commit `66e0f9273` was updated after the
source-ready marker, so its document hash differs from the marker's historical
document hash. Both historical marker and later document are preserved, not
rewritten to look identical. Compiler source and driver identities match.

Original absolute physical paths and base64 diagnostic bytes are unchanged.
Compiler binaries, fixture executables, and incremental caches are omitted.

## Results, without retcon

The original fixture passed **672** semantic observations but failed its required
mechanism proof: cold/warm on each had **23 traversal scopes, zero cross-root
revisits**, while off had zero markers. Its three roots were in different initial
CGUs. Final CGU merging happens after traversal, so final CGU count one could not
supply the missing same-initial-CGU root sharing. This remains an explicit failed
positive-proof calibration, not a compiler failure or a successful native proof.

Moving root_b into module `a` beside root_a, while retaining root_c in module `c`,
produced a positive focused four-process calibration. The repaired full root run
then passed **672 semantic observations plus four independent mechanism processes**.
Both focused and repaired full profiles record cold/warm on each **23 scopes and
one cross-root revisit**, off zero. The marker counts and required proof were not
weakened, nor were native marker emission or compiler source changed for the repair.

Driver SHA-256: `29ea709935d751ce13e77995173659006c1e15683e0f0645616d12cd5a3a1665`.
Executable SHA-256: `570faeb27338cea8625c4a4a7eaeef71fc75470508657ff489c3da96ce5f9144`.
All original, focused, and repaired report identities match before/after.

## Fully detached offline replay

Copy this directory alone to any location with Python and run:

```sh
python3 audit.py
python3 test_audit.py -v
```

No git checkout, original scratch directory, compiler, fixture executable,
network access, or third-party Python package is required. The audit imports
only archived Python source snapshots, compares both complete 672-mode matrices
against the correct original/repaired fixture revision, checks exact diagnostics,
statuses and runtime expectations, validates tracked transitions and persistent
cache continuity, verifies nonincremental configurations omit incremental flags,
and checks all compiler identities and frozen compiler-source hashes. All twelve
raw profiles are reparsed to verify hash, size, metadata, marker counts, and unknown
labels. The original zero-revisit failure is asserted precisely, then both positive
proofs are validated. Ten mutation tests reject corrupted evidence and copy only
this bundle's audit plus archives into a new directory for isolated Python replay.

Native performance measurement is separate. These acceptance results do not
justify a latency improvement claim or enabling the option by default.
