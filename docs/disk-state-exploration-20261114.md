# Disk-backed compiler-state exploration, 2026-11-14

## Question

Can saving compiler work to disk and restoring it in parallel beat recompiling, and is a more ambitious disk tier worth implementing?

## Preliminary recommendation

Preserve and reuse disk state: yes. Treat bulk extraction using every core as the main optimization: not yet. Existing Rust incremental persistence already makes a large difference. The remaining compiler work and the breadth/precision of saved state are larger opportunities than shaving a few milliseconds from copying warm files. These are focused hosted experiments, not a comprehensive workload result or production promotion.

## Hosted check experiment

Actual pinned justrust source archive, `check --lib --locked`, incremental enabled, outer artifact caches disabled, one changed function body per timed call. The initial preparation compiled dependencies and is excluded. Each row then rebuilt exactly one local unit with dependencies fresh. Existing incremental state was either removed, or restored byte-for-byte from the same baseline snapshot with 1/4/16 file-copy workers. Three rounds rotate mode order. Edits have identical structure but distinct integer constants to ensure the sync layer cannot skip unchanged bytes.

Source revision: `63f1c408bc902cd818a3d6389baac5a3ebb4c064`. Hosted toolchain reported `jr-ac20de5e7921`, rustc `1.101.0-nightly (ac20de5e7 2026-10-11)`. The account microVM has 16 vCPUs and 28.9 GiB RAM. Other builds were active, so observed small differences between worker counts are not attributed causally to copy parallelism.

The snapshot contains **147,770,326 bytes in five files**: about 77 MiB dependency graph, 56 MiB query cache, 9 MiB metadata, and tiny work-product/lock files. This is selected compiler state, not a full process heap or every LLVM intermediate result.

| Mode | Median compiler interval | Median copy time | Median host wall + each sample's copy time | Median client wall + each sample's copy time |
| --- | ---: | ---: | ---: | ---: |
| Incremental state absent | 13.329 s | 0 | 13.442 s | 14.879 s |
| Restore, 1 worker | 3.875 s | 0.0719 s | 5.055 s | 6.694 s |
| Restore, 4 workers | 3.314 s | 0.0475 s | 3.508 s | 5.890 s |
| Restore, 16 workers | 3.609 s | 0.0482 s | 3.823 s | 9.432 s |

The four-worker case has an observed **4.02x compiler interval ratio**, **3.83x host-plus-copy ratio**, and **2.53x client-plus-copy ratio**, versus absent incremental state. These are descriptive median ratios on this workflow, not gains over ordinary incremental Rust or evidence that four workers cause faster compiler execution. The serial row includes a 1.291s first restore outlier; medians of paired sums need not equal sums of separate medians. Client overhead also varied substantially, especially in the 16-worker rows.

Copy and hash verification are outside the compiler interval. SHA-256 verification is outside the copy timer and warms the restored files, so these initial compiler/copy results are warm-page-cache measurements, not cold SSD results. Client-plus-copy excludes the administrative SSH round trips used to arrange the experiment and correctness hashing. A production restore would run on the host, but its validation cost must still be included if it requires full hashing.

Raw receipts: `bench/results/disk-state-hosted-check-20261114.json`. Harness: `bench/hosted_disk_state.py`. Private stdout/stderr and original source are retained under the run's scratch directory.

### Where the remaining time goes

Compiler phase records from the first round show:

* No incremental state: type checking 6.437s, expansion 0.881s.
* Restored state, four workers: type checking 0.241s, expansion 1.011s.
* Restored state, sixteen workers: type checking 0.231s, expansion 0.880s.

Expansion includes resolution in nested activities. Do not add nested phase times. This points toward retaining more frontend work, not merely faster copying. It does not prove all expansion can safely be skipped.

## What the current compiler already does

Inspected the owned compiler source, not just a conceptual cache model:

* `toolchain/rustc/compiler/rustc_incremental/src/persist/load.rs` opens incremental files as memory maps and constructs the on-disk query cache.
* `toolchain/rustc/compiler/rustc_middle/src/query/on_disk_cache.rs` retains mapped serialized bytes, decodes the footer/index and lazily populates decoded state. This is already a demand-driven disk-backed design, not blanket reconstruction of every saved value.
* `toolchain/rustc/compiler/rustc_middle/src/dep_graph/serialized.rs` reconstructs dependency graph arrays from serialized nodes. Parallelizing or redesigning this representation is a narrower hypothesis than blindly unpacking everything.

These source observations do not prove that a particular loading routine dominates the installed toolchain's critical path. Current phase receipts do not isolate all load/decode time.

## Limits and scope

No new compiler persistence format or general parallel query decoder was implemented. Byte-for-byte copying and zstd extraction are transport/materialization experiments. They cannot establish that arbitrary live compiler arenas or LLVM state can be serialized or soundly reused after edits. Fingerprints, compiler/schema identity, dependency invalidation, diagnostic effects and ownership remain required.

A first backend attempt with an explicit target-dir was refused by the existing build router before compilation. It is not a performance sample. The supported retry disables slots only in the private archived workspace and uses its normal target directory. No production defaults, tenant limits, billing/admission checks or laptop local-build policy were changed.
