# Existing-run slim native gate exports

These nine summaries were exported read-only from the original v31/v32/v33 check/build/full359 journals, not newly executed workloads:

```sh
JUSTRUST_HOME="$ROOT/candidate-$kind-v$v/justrust-home" justrust export --out "$BUNDLE/slim" --machine "native-v$v-$kind"
```

`ROOT` was `/home/jeremy/.jcode/scratch/blaze-native-engine-20261030`, `v` was 31, 32, 33 and `kind` was build or unit. The exporter classified all nine as private and redacted paths/crate names/arguments. Exact commands, native toolchain paths, raw libtest names and source manifests remain in `../journals/`. `../validate.py` links the nine IDs and results, including each full359 count. Slim summaries do not replace raw correctness oracles. No ambient environment or binary/cache tree was exported.
