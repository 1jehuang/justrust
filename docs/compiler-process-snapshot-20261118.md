# Hosted full-process compiler disk snapshots, November 18, 2026

## Scope and mechanism

This is a focused feasibility experiment, not a default promotion or a general compile-speed claim. It checkpoints an actual warmed native rustc worker with upstream CRIU, terminates that original process, then restores its address space and resources from image files before compiling changed source. It is not an encoded query cache or a fork-only memory checkpoint.

The candidate is the previously frozen managed-decoder compiler, not the Blaze-derived engine. Its launcher SHA256 is `8cb4b7089e2e70cbd4832fdf53a24ef2e27db676dcc3632c2501134e6f3dd567` and its actual rustc driver SHA256 is `4289362d41b6bff5df8caa0c95bad6311fbccd6b7f82422c32a7f6d3b9c2d598`. Optional AST, lexer, typed-query and managed-decoder retention are disabled. Consequently this tests process-state restoration, not preservation of a whole live semantic database or a Blaze-equivalent edit loop.

The dedicated retained experiment host is an m7i.4xlarge, with 16 logical CPUs, about 61 GiB available RAM, and Ubuntu 24.04 on kernel `7.0.0-1017-aws`. CRIU v4.3 was built on that host from upstream commit `2b7ac603cda31d015d91c856c34c557cb2300e7d`. Its binary SHA256 is `38068177d6eb119c627b63058eee8eb04356bae4010999307fc2503dc207d3a0`. No native workloads were run locally and the installed production client was not replaced.

## Correctness contract

`bench/compiler_process_snapshot.py` drives fresh, continuously warm, and disk-restored compiler processes over the same seven transitions. These cover unchanged source, root and submodule body changes, a signature type error, and recovery. Each checkpoint precedes installing the next source edit. The dump does not leave the original worker running. Restore must recover the expected PID and executable. Named FIFO request/reply channels avoid external anonymous-pipe ownership at checkpoint time.

Every transition checks exact fresh status and diagnostics, runtime output with a new nonce, and byte-identical produced executables. Compiler identities must remain unchanged. Checkpoint inventories retain image sizes and SHA256 hashes. Cleanup kills only the experiment's verified workers and restores its private source files. Unsupported CRIU, failed dump/restore, or mismatches retain failure evidence rather than substituting a warm worker.

The initial real hosted capture passed all seven transitions. Its untouched original receipt is authenticated and losslessly reconstructable from account-private record `compiler-process-snapshot-20261118-first-0712`, SHA256 `447928afa91537aebad7600ccbefdee1a50b0bf8e52c83639b7b06d40a1efab5`.

## Interpretation limits

Restore reads freshly written image files, normally from the OS page cache. This is not a cold physical-disk latency test. Fresh means a new compiler process, not a cold filesystem or dependency cache. Dump cost is separate from restore-plus-compile and must be amortized if snapshots are used across idle periods. Root-controlled checkpointing on an isolated host is not a production multitenant checkpoint service.

Additional available cores do not imply parallel restoration. Disk throughput, page faults, CRIU's reconstruction path and the compiler's own work can dominate. The affinity experiment below measures this limited fixture rather than extrapolating to large projects.

## Repeated hosted measurements

Five independent captures were run for each of two CPU affinities, first with `fresh,warm,restored` mode order, then five more with `restored,warm,fresh`. Affinity groups alternate first position between replicates. Each capture performs all seven transitions. All **20 captures and 140 transition comparisons passed**, including the signature error and recovery. The initial feasibility capture is additional, not included in the table.

The first order used frozen harness commit `3ad0a93cc`; the reversed order used `a937324d4`, which adds reporting/provenance, explicit order, repeats, and mocked orchestration coverage. Compiler bytes, helper workload, arguments and strict correctness gates are unchanged. Orders are reported separately rather than silently pooled. Twelve pure Python tests pass. The reverse-order receipts record boot ID and helper/compiler/CRIU identities. The archived SSM commands bind requested CPU affinity through `taskset`; these original receipts did not separately record effective affinity. A subsequent reporting-only fix adds `cpu_affinity` to future receipts without rewriting these originals.

Each cell below is the median across 35 transition observations, in milliseconds. This includes the error transition, so these are descriptive fixture medians rather than a session-weighted workload score.

| Available CPUs | Mode order | Fresh process + compile | Live warm compile | Disk restore + compile | Dump only | Restore command only |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| 1, pinned CPU 0 | fresh first | 58.088 | 42.397 | 60.217 | 44.670 | 17.839 |
| 1, pinned CPU 0 | restored first | 57.247 | 42.579 | 62.375 | 45.285 | 17.160 |
| 16, CPUs 0-15 | fresh first | 62.946 | 45.174 | 66.036 | 48.288 | 19.035 |
| 16, CPUs 0-15 | restored first | 60.534 | 45.657 | 68.906 | 50.135 | 19.098 |

Image inventories ranged from about **15.5 to 39.9 MB per checkpoint**, increasing across revisions. Restore-plus-compile includes the restore command and the subsequent actual compiler request, but not the preceding dump. Dumping and restoring on every edit would therefore be substantially worse than either fresh or warm execution here. Individual phase results remain available in the raw receipts; for example, in the 16-CPU fresh-first group the type-error medians are 29.779 ms fresh, 12.693 ms warm, and 32.365 ms restored, while recovery is 63.162, 45.384, and 71.153 ms respectively.

**Result:** real process snapshots work correctly for this fixture, but they do not improve edit latency over a fresh compiler process. Keeping the worker alive is faster. Sixteen available CPUs did not improve restoration compared with one pinned CPU. This does not establish that more cores are harmful generally, only that this restore path and small fixture did not use them to produce a speedup.

The useful next hypothesis is checkpointing a much more expensive, semantically populated worker to reclaim idle RAM, then amortizing the dump over many future requests. That remains untested. We should not replace the live worker or promise a 10x gain based on this experiment. Larger semantic state, idle-time checkpoint economics, cold-disk resume, kernel/toolchain upgrades, resource rebinding and tenant isolation need separate gates.

## Evidence and operations

The actual affinity commands are archived as `compiler-process-snapshot-20261118-matrix-command` and `compiler-process-snapshot-20261118-reverse-command`. All 21 original receipts, request/reply transcripts, stdout/stderr and CRIU dump/restore logs are retained in the lossless raw archive described below. Page image bytes are retained on the experiment disk, not uploaded to the record store; every image's size and SHA256 is bound in its original receipt. The failed first attempt to collect root-owned logs as the unprivileged SSH user was not used as the evidence archive. Authenticated SSM collected the complete archive without changing the underlying logs or broadly relaxing permissions.

The complete archive is 9,451,013 bytes, SHA256 `e36f7a402a6471d2f29f27bb4bff439e8d213e4f885454314f3ba922fda525a3`. It exceeds the existing single-record bound, so it is stored as ten exact binary shards under `compiler-process-snapshot-20261118-raw-raw-part-00` through `compiler-process-snapshot-20261118-raw-raw-part-09`. Shards 00-08 are 1,000,000 bytes each and shard 09 is 451,013 bytes. Every shard was fetched through the authenticated record API and verified byte-for-byte. Concatenate reconstructed shards in numeric order and check the full archive SHA256 before extraction. This preserves all original receipt and log bytes without changing backend limits.

The dedicated instance was stopped and its stopped state verified after archival. Its retained encrypted disk preserves the page images. No production compiler defaults, installed client, billing or tenant isolation were changed. Source, tests and this report are committed on `stable`.

