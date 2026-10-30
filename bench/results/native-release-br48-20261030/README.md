# br48 optimized native compiler frontier

Portable offline replay with Python 3.11+, no native executables:

```sh
python3 validate.py
python3 test_validate.py
```

Seven stacks, 35 accepted edits, 70 rows, 50 owned private recorder states and
50 dedicated exports. Exact runtime, negative, recovery, source/tool/seed,
backend/thread/profile flags and medians are independently replayed.

Plain optimized compiler: ThinLTO, CGU1, nonincremental compiler build.
NOT PGO/BOLT. Executable SHA-256 starts `64689a080e88a1ae`, driver SHA-256
starts `961a7344a71a7b19`. Full pins are in validator and retained identity files.
The two retained source files match commit66e0f9273 and build before/after hashes.
`release-build/` contains config/build/status/version/identity/source receipts,
complete native acceptance JSON and all four raw mechanism profiles.
`acceptance-runner/` contains the pure replay helpers and format-9 raw decoder.
No compiler/driver/fixture binaries, sysroot, targets, caches or sockets retained.

Native acceptance: 96 cases, eight configurations, 672 main compile modes plus
four mechanism compiles/profiles PASS. Cold/warm off marker counts are0/0,
on counts23/1 (traversal/revisit). Raw profiles are decoded during replay.
This correctness/mechanism evidence is not performance evidence.

Medians, seconds: owned-off1.028383, all3on0.997915, native-clif1.057973,
threads4 0.966622, clif+threads4 0.994295, Blaze0.099186, BlazeLLVM0.109765.
No significance, general performance or default-promotion claim. No controlled
historical dev-versus-release speed claim. Seeded-warm four-filtered-test only.

`state/` and `exports/` retain fifty dedicated calls, not a global export.
`sha256.json` seals all retained files including validator and mutation sources.
`seal.py` refuses to overwrite it. Semantic mutations update raw hashes/copies
and raw-profile hashes, so independent runtime/native marker checks must fail.
The complete offline audit is also tested with process creation forbidden.

Absolute paths in artifacts are historical strings only. No private files or
repository context are required for copied-directory replay. Use normal Python,
not `python -O`. `retain.py`, launcher, and build/capture shell scripts are
historical utilities, NOT replay commands. No native workloads were run during
retention. Full protocol: `docs/native-release-br48-20261030.md` in the repository.
