# Indexed expansion phases: experiment, not the shipping recorder

`indexed-expansion.patch` changes only the phase ledger and the common guard for six expansion labels. Apply it at the repository root with `git apply`. It leaves measureme's generic activities intact. An enabled ledger records every event with the same two `Instant` reads. The experiment replaces a string hash lookup with a fixed slot update under the ledger mutex. There is no per-thread state waiting to flush. Snapshots merge any ordinary additions to the same name, omit unused slots, retain all state across early writes, and feed the existing TSV formatter.

In a **patched** compiler, use `JUSTRUST_EXPERIMENT_INDEXED_PHASES=1` with `JUSTRUST_RUSTC_PHASES_FILE` to select indexed rows. Without applying the patch, the compiler is unchanged.

To reproduce from `toolchain/rustc`, with the existing development bootstrap configuration:

```sh
./x build --stage 1 compiler/rustc library
# Save the complete unpatched build/host/stage1 sysroot, apply the patch, rebuild.
./x test --stage 1 compiler/rustc_data_structures --test-args profiling::
./x test --stage 1 compiler/rustc_data_structures --test-args 'six_label_guard_cost --ignored --nocapture'
python3 compiler/rustc_data_structures/src/profiling/experiments/compare_regex_syntax.py \
    /path/to/original/bin/rustc /path/to/indexed/bin/rustc /path/to/regex-syntax-0.8.8/src/lib.rs
```

**Decision: do not promote.** On the development stage1 compiler, the full guard microbenchmark fell from 55.68 to 44.08 ns/event (16 paired runs, median paired saving 11.82 ns). The hosted accumulator microbenchmark independently fell from 63.94 to 59.57 ns/event (run `20261119-015822767-3538208`). But the real regex-syntax metadata build, pinned to CPU 3, took 1040.82 ms baseline versus 1046.05 ms candidate across 16 measured pairs. Its median paired saving was negative (-4.42 ms), with individual savings from -810.64 to +20.24 ms. Macro expansion itself took 65.25 versus 66.71 ms. All phase counts matched, including 3,009 events across the four exercised expansion labels. The all-six microbenchmark and concurrent early-write/rearm tests cover the other labels and lifecycle. This is not evidence of a user-visible speedup.

The microbenchmark alternates the order of the actual guards in 16 pairs of fresh processes and checks all 1.2 million counts in every arm. The workload benchmark discards two warm-up pairs, verifies every phase count, checks metadata determinism per compiler identity, and prints per-pair timings. To isolate only the storage choice and require byte-identical metadata, pass the same patched compiler in both positions. To compare the full implementation, pass the actual original and patched compilers.
