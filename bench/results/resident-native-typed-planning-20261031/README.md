# Rejected resident native typed planning v83 evidence

**REJECTED / NOT PROMOTED.** This is licensed Blaze-derived resident code, not the clean-room compiler. Production source is restored to accepted `c8829bae0`. No native builds, probes, tests or benchmarks are executed by this bundle's offline validator.

## Result

Root's actual native named gates passed 22 manifest, 4 metadata, 53 codegen and 373 resident tests. Eight ON activation/profile rows and 28 OFF/ON/ON/OFF strict transition rows passed. The pinned comparison retains all 54 calls, 20 measured samples, 24 warmup/sample nonces, all first touches and worker PID/start-tick/executable/socket identity.

OFF median **0.054038845002651215 s**, ON median **0.05164486599096563 s**, descriptive reduction **4.430107659717997%**, below the requested 10% floor. Noise is unknown. Whole-generation and watcher remain ON in both candidate arms, reference all flags OFF. This is combined same-byte manifest-plus-lock retouch only, not causal attribution, significance, general speedup, stock-versus-resident evidence, or default promotion.

Summary preparation in the ON profile spans 1.968 to 3.344 ms, inclusive diagnostic, not an independently controlled before/after speed claim. Five ABBA stops and four post-measurement strict-namespace stops succeeded. Stores were retained.

## Contents and provenance

- `source/`: six full accepted originals, eight full rejected candidates, sole cumulative/incremental patch, immutable source-only release/check receipts and explicit restoration receipt. All eight live outputs were preserved before writes. Six production files were restored through the write tool and compared byte-for-byte to originals and HEAD. The only removed file was the archived new nested test module.
- `gates/`: explicit recorder fields and complete `t/s/l` output journals, including all four authored metadata test names. Source inventory 3,758 files, toolchain inventory 4,020 entries, root lock preservation and engine lineage receipts join the candidate under test.
- `oracle/`: unchanged accepted original pure oracle definitions. `validate.py` loads only named definitions through AST extraction, not whole archived imports or drivers.
- `harness/ancestor/`: immutable v81 suite and tests. `harness/typed/`: exact derived v83 factor source, bound to its executed SHA. `drivers/`: inert historical scripts, not executable by offline replay.
- `harness/typed-factor-v83-initial-mock-failure.log` records the initial failed negative test. Positive marker renaming made a test-only `v81` to `v80` mutation ineffective. The corrected adapter changes that mutation to `v83` to `v80`, leaving the negative assertion unchanged. Corrected 18 mocked tests passed, zero native jobs. This is not assertion weakening or a native rerun.
- `profile/`, `strict/`, `abba/`, `cleanup/`: selected raw text and allowlisted process records. Profile traces retain exact selected complete-event timing fields, while original full trace SHA/size is recorded in `private-inputs.json`.
- The inherited strict state scope says `parallel manifest syntax`; that label is intentionally retained as provenance. Actual source/ELF/flag/raw marker identify native typed v83, not an activated parser experiment.

Registry/git fixtures cover real filesystem declarations and exported pinned source conversion. They do **not** prove full registry/git fetch integration. Complete Summary comparison explicitly includes dependencies, features, links, checksum, rust-version, publication time and all Read fields, never Summary's ID-only equality.

## Offline verification

From the repository root:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 bench/results/resident-native-typed-planning-20261031/validate.py --manifest
PYTHONDONTWRITEBYTECODE=1 python3 bench/results/resident-native-typed-planning-20261031/test_offline.py -v
```

These replay preserved observations and original oracles only. The negative suite coherently reseals its temporary copies before rejecting semantic corruptions. An isolated `python -I -S -B` subprocess with empty HOME and PATH denies outside reads/listings, writes, subprocess execution, sockets and ctypes loading after importing stdlib. This subprocess runs only Python evidence validation, never the engine or compiler.

`seal.py` is an explicit maintainer hashing utility, not semantic proof. `HISTORICAL.json` and fixed observations commitments prevent silently rewriting the historical corpus. Semantic checks run before fixed-history commitments, and negative tests explicitly bypass only history commitment checks to prove they are not rejected merely by checksums.

## Privacy and boundaries

`export_private.py` is an author-only exporter. It reads only explicitly named private inputs and selected immutable Git text. It emits field allowlists, no ambient environment, and uses caller-supplied exact private-prefix role redactions. Every read private input retains original SHA-256 and byte size, then is rechecked unchanged after export. No binaries, secrets, private absolute paths or runtime stores are included. Original private inputs remain locally retained. Exposing fixture source and license lineage is intentional.

The worker's actions were source implementation, exact restoration and offline evidence processing only. Root performed the native gates and protected runtime measurements. No active source change, release or production behavior promotion is part of this package.
