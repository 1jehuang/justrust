# Disk-backed compiler-state exploration, 2026-11-14

## Question

Can saving compiler work to disk and restoring it in parallel beat recompiling, and is a more ambitious disk tier worth implementing?

## Recommendation

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

## Direct disk-to-RAM loading and decompression

A second, read-only experiment avoids copying restored state back to disk. It loads existing immutable snapshot bytes into anonymous RAM buffers with independently readable 4 MiB chunks, then verifies every chunk's SHA-256 outside timing. Each mode has three samples at 1/4/16 workers, with alternating worker order. No compiler executes during these byte/codec measurements.

For guest-cold rows, `POSIX_FADV_DONTNEED` targets only our private immutable snapshot before timing. `/proc/self/io` confirms guest storage reads: a median 147,779,584 bytes for the 147,770,326-byte snapshot and 696,451,072 bytes for the 695,913,032-byte snapshot. Padding/readahead makes the accounting slightly larger than file sizes. Host filesystem/device caches remain uncontrolled, so these are **guest-cold reads**, not physical-device cold benchmarks. Warm rows record zero storage-read bytes.

| Snapshot and operation | 1 worker | 4 workers | 16 workers |
| --- | ---: | ---: | ---: |
| 148 MB check state, warm load into RAM | 62.9 ms | 19.8 ms | 13.9 ms |
| 148 MB check state, guest-cold load | 0.767 s | 0.550 s | 0.381 s |
| 148 MB check state, decode zstd frames already in RAM | 116.6 ms | 49.0 ms | 55.9 ms |
| 696 MB codegen state, warm load into RAM | 0.280 s | 0.093 s | 0.100 s |
| 696 MB codegen state, guest-cold load | 12.912 s | 6.038 s | 5.702 s |
| 696 MB codegen state, decode zstd frames already in RAM | 0.546 s | 0.317 s | 0.319 s |

All cells are medians. The larger snapshot includes real generated object files from a successful hosted `build --lib` preparation: 261 files and 376 chunks. It is not a full LLVM heap checkpoint. The smaller snapshot contains 38 chunks.

Parallel raw loading improved the observed guest-cold median about **2.01x** for the smaller snapshot and **2.26x** for the larger snapshot. Four workers already achieve most of the large-snapshot loading benefit. Zstd decompression improved about **2.38x** with four workers for the smaller state and **1.72x** for the larger state; sixteen did not improve those codec medians. This supports a bounded/adaptive pool rather than assuming every core should decode.

### Compression cost and capacity

Real zstd 1.5.5, level 3, independent 4 MiB frames:

| Snapshot | Original bytes | Compressed bytes | Space reduction | One-time encode wall |
| --- | ---: | ---: | ---: | ---: |
| Check state | 147,770,326 | 52,845,782 | 64.24% | 0.433 s |
| Codegen state | 695,913,032 | 186,463,291 | 73.21% | 1.497 s |

The codec rows above decode **RAM-resident compressed frames**. They do not include reading those compressed frames from SSD, persistence cost, or reconstructing compiler semantic values. Do not report 0.317s as a complete 696 MB cold restore. A roughly bandwidth-proportional reduction in storage time is a hypothesis, not a measured compressed-SSD result.

The measurements ran on a busy shared guest: recorded load averages at the start were about 103 for the check-state read experiment and 67 for the codegen-state experiment, on 16 vCPUs. This is useful deployed-system evidence, not an isolated-machine performance promise. Cold-load results are especially sensitive to shared storage contention.

Raw receipts:

* `bench/results/disk-state-memory-check-20261114.json`
* `bench/results/disk-state-memory-build-20261114.json`

Harness and local protocol tests: `bench/hosted_state_memory.py`, `bench/test_hosted_state_memory.py`. The helper hashes are retained in each receipt. The check-state run preceded the later empty/symlink input guards, which do not change timed work. Both runs verified all restored chunks; the full owned harness suite passes **38 tests**.

## Incomplete backend and write-heavy attempts

The broader backend compile comparison prepared a valid 695,913,032-byte snapshot, then completed only six of twelve planned edit rows before the background runner's 600-second limit. Early examples ranged from 26.13s compiling without incremental state to 5.57s with restored state, but later restored compiles took 28.71s and 72.97s under contention. One client call spent 134.5s waiting for another workspace's source sync. One copy took 29.9s instead of the earlier 0.159s. **No completed backend median speedup is claimed.** Partial receipts: `bench/results/disk-state-hosted-build-20261114-incomplete.json`.

The file-materializing I/O harness completed a warm case (median copy 68.2ms serial, 48.2ms at four workers, 45.6ms at sixteen), but its overall setup took minutes. A remote process inspection found it blocked in `balance_dirty_pages` while guest load exceeded 100. The subsequent guest-cold/write-heavy attempt was cancelled rather than promoted as clean evidence. The process was confirmed gone; no other build was stopped. Partial receipts: `bench/results/disk-state-io-check-20261114-incomplete.json`.

These results favor **loading state directly or mapping it in place**, rather than eagerly copying entire caches into new disk trees on every edit. They also show why multi-user I/O admission matters independently of compiler caching.

## Is it worth implementing?

1. **Existing disk reuse is clearly worth preserving.** The completed check experiment shows roughly 4x compiler-side benefit over throwing its incremental state away. This is not a new 4x improvement over today's correctly warmed incremental path.
2. **Saving more expensive reusable state is worth a scoped prototype.** The measured smaller snapshot can be fetched from guest-cold storage in about 0.38s, while the remaining check compilation is several seconds. The missing experiment is whether broader state can be safely encoded, validated and reconstructed cheaply enough. Raw read time alone cannot answer that.
3. **Compressed cold storage is promising, not a proven default.** Real snapshots shrink 2.8–3.7x and decode in 0.05–0.32s with four workers once frames are in RAM. Encoding costs 0.43–1.50s, so compressing the whole cache on every edit could regress the hot path. Prefer background compression on eviction/idle transitions, with versioned independent chunks and measured benefit per retained byte.
4. **Do not build an all-cores bulk-unpacker first.** Use lazy, demand-driven state with a bounded pool. Extra threads beyond four produced little or no large-state/codec gain here, and they compete with actual compilation and other accounts.
5. **Keep three tiers:** hot reusable state in RAM, directly usable/mapped local disk state, compact cold snapshots. Add strict tenant/config/schema identities, sound invalidation and memory/I/O budgets. Neither raw bytes nor stable-looking fingerprints justify retaining stale compiler answers.

## Limits and scope

No new compiler persistence format or general parallel query decoder was implemented. Byte-for-byte copying and zstd extraction are transport/materialization experiments. They cannot establish that arbitrary live compiler arenas or LLVM state can be serialized or soundly reused after edits. Fingerprints, compiler/schema identity, dependency invalidation, diagnostic effects and ownership remain required.

A first backend attempt with an explicit target-dir was refused by the existing build router before compilation. It is not a performance sample. The supported retry disables slots only in the private archived workspace and uses its normal target directory. No production defaults, tenant limits, billing/admission checks or laptop local-build policy were changed.

After exporting receipts, the experiment's hosted targets, snapshots and temporary I/O trees were removed. Cleanup first checked that no process was running inside the archived workspaces. Source archives, logs and small manifests remain in scratch, and `bench/hosted_disk_state.py` can regenerate the snapshots. Unrelated targets and builds were untouched.
