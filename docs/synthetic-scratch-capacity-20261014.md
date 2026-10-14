# Scratch capacity required by the unchanged synthetic comparison

Date: 2026-10-14. This is an offline storage-boundary map, not a capacity forecast or a completed performance comparison.

## Why cleaning between measured points is not the fix

`bench/synth/run.py` selects one retained target directory per snapshot and stack. The pre-build, optional previous command, and measured command all use that same target. The pre-build establishes pristine-source state, edits are applied, the optional previous command establishes a command-switch state, and only then is the command measured. Sources are restored afterward. Target artifacts are retained across points and same-stack suite runs.

`bench/suite.py` deliberately names the single stack by selected toolchain. `bench/versus.py` separates each stack's private dependency cache. `bench/synth/gen.py` uses different stock-probe target directories and caches probe tables separately. These stores have different roles:

| Store | Required boundary |
|---|---|
| Source snapshots and restoration journal | Preserve pristine source and interrupted-edit recovery |
| `synth/probe` tables | Preserve the selected generator's exact live-edit locations |
| `synth/probe-target` child Cargo roots | Unmeasured stock-probe outputs, separate from measured targets |
| `versus/target/<snapshot>/<stack>` | Retain pre/previous/measured state and cross-point warm history |
| `versus/cache/<stack>` | Retain the selected stack's private dependency-cache history |
| Private toolchains, runs, receipts and result logs | Preserve compiler identity and reconstructible evidence |
| Existing service/user caches outside the disposable experiment | Never reclaim to force a benchmark result |

Removing measured targets or the private dependency cache between points would change the experiment. It is not a storage-only implementation detail. Cleaning a parent container lacking `CACHEDIR.TAG` must remain refused. The existing attempt reclaimed only its three new, tagged, unmeasured stock-probe child roots after preserving the tables.

## What the observations do and do not size

The attempt had approximately 18 GiB free after private setup. Stock probes reached the 8-GiB safety threshold. Reclaiming approximately 11 GiB of its unmeasured probe outputs allowed one measured OFF point and the next pre-build, but the same guard stopped again. See `docs/scheduler-comprehensive-2172-20261014.md` and its three exact command records.

These observations do not measure the final retained footprint of all 40 points or the ON epoch. They cannot establish that a particular larger volume will suffice. Peak memory, CPU count, and one crate's output size are not substitutes for an actual filesystem-capacity observation.

A safe planning inequality is:

```text
free bytes at phase entry >= 8 GiB reserve + peak additional bytes still needed
```

The second term must cover newly accumulated measured targets, private cache growth, other profile/command outputs, source/toolchain staging not already present, and logs. It remains unknown for the complete workload. A preflight free-space observation alone is not a guarantee, so the per-invocation guard must remain enabled.

## Bounded check performed

An offline harness exercised the existing `_run` target-selection boundary three times with pre/previous/measured tags. All three selected the identical target. Changing snapshot or stack changed the target path. Child compilation was stubbed, temporary directories were isolated, and no compiler or account-host operation ran. This validates path/lifecycle behavior only, not disk growth or complete workload acceptance.

## Safe prerequisite for the next full trial

Provide a separately arranged sufficiently capacious disposable benchmark filesystem or host, verify the effective guest filesystem size rather than only an EBS configuration value, and preserve the fixed client/Cargo/compiler/hint hashes and workload identity. Keep caches and execution mode comparable across OFF/ON. Screen foreign work and retain noisy observations without turning them into performance claims.

The read-only inspection of current worker/bootstrap permissions did not verify an authorized online growth path. It did not prove that AWS cannot resize storage or that an administrator cannot arrange it. This follow-through changes no volume, reboot, IAM, sudo, hosted default, cache or measurement command. The ordinary account host remains released. The complete matched performance gate is still blocked, not completed by this map.
