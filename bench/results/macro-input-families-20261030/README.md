# Frozen proc-macro cache failure evidence, October 30, 2026

This bundle permanently retains the **failing** v2 native correctness report
from the frozen experimental compiler, not a run of the later safe-default
repair (`e5bc943e9`). No native compiler workload was run while packaging it.
The original report bytes are unchanged, including historical absolute paths.

## Portable validation

From any working directory, with only Python 3's standard library:

```sh
python3 /path/to/this/directory/validate.py
```

Expected result: `evidence-valid`, eight stale cases, ten named disk-load
probes, two passing cached constant cases, and five passing uncached controls.
This means the **failure evidence is intact**, not that the compiler is safe.
The validator opens only paths relative to this directory. It requires neither
the original scratch directories, compiler, repository checkout, nor profiler.
It rejects altered or missing artifacts and checks every compile/runtime matrix
observation, exact family coverage, query-specific incremental load, and
zero provider invocations in the ten second-process probe summaries.

## Contents and provenance

- `report.json`: byte-for-byte full report originally at
  `/home/jeremy/.jcode/scratch/macro-input-families-v2-20261030.json`.
  SHA-256: `a5aa0c43d1d13d7506c11a4005955e3e396167046cac645b49885442ff628dfd`.
- `identity.json`: report-linked launcher and driver SHA-256 identities,
  compiler `-vV`, original fixture root, and old root report's limited scope.
  Compiler binaries are not bundled. This preserves original attestations,
  rather than attesting an installed compiler on the validating machine.
- `probes.json`: portable artifact receipts for all ten named-load probes.
- `probes/{derive,attribute}-{constant,env,tracked_env,file,tracked_path}/`:
  each contains `warm.mm_profdata.gz`, `body-edit.mm_profdata.gz`, and the full
  second-process `analysis.json`. Gzip compression is lossless and deterministic.
  Both compressed and original uncompressed trace bytes are hash-checked.
- `sha256.json`: complete data-artifact integrity manifest.
- `validate.py`: offline standard-library validator with pinned original report
  and frozen compiler hashes. No subprocesses, compiler, or network access.

For independent profiler reanalysis, decompress a trace outside this directory
and run a compatible `summarize summarize --json` on the resulting
`.mm_profdata`. The portable validator verifies retained analysis against the
full report and hashes the raw traces. It does not itself reimplement the
measureme binary decoder or rerun profiler analysis.

Validation was exercised on October 30, 2026 both in this checkout and on a
detached copy under an isolated `python3 -I` process, with working directory `/`
and a minimal environment. The intact copy validated. Appending one byte to
`report.json` was rejected, and deleting a compressed profile was rejected.
Only the detached test copy was mutated. No native workloads were executed.

## Safety conclusion and limits

Persistent cache-on reuse returned `alpha:0|2` instead of `beta:0|2` for both
**derive and attribute**, across ordinary environment/file reads and supported
modern tracked environment/path APIs. The other five configurations observed
the expected changed value. Constant and macro-crate-rebuild transitions passed.
All five functionlike controls passed, but those macros do not exercise the
frozen compiler's cached expansion families. The earlier root PASS was only
that limited uncached negative coverage.

The ten named-load probes are dedicated warm/body-edit pairs, not profiles of
every failing matrix transition. Each second process has positive incremental
load time for its precise `derive_macro_expansion` or `attr_macro_expansion`
query with zero provider invocations. This is evidence of actual native disk
reuse in those probes, not a speedup claim or proof that all transitions hit.

This evidence rejects safety of the tested frozen cache for the recorded input
transitions. It neither tests nor invalidates the later client safe-default fix,
which keeps unsafe proc-macro expansion caching explicit opt-in. It is not a
claim about all compilers or macros, Cargo dependency integration, arbitrary
side effects, concurrency, or absent/deleted inputs.
