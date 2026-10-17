# Bounded shared-byte-slice runtime contract

These exact-source fixtures are a **bounded prerequisite** toward adapted Adler2
kernels. Passing them is not unchanged Adler2 acceptance, full Rust support,
public Rust ABI compatibility, or a performance improvement claim. The separate
Adler2 acceptance runner owns checksum vectors. Compiler diagnostics and rejection
fixtures are deliberately outside this runtime runner.

## Expected outcomes

* `positive.rs`: successful check, build, and execution with stdout exactly
  `slice-contract-runtime:ok\n`.
* `bounds-empty.rs`, `bounds-equal.rs`, `bounds-max.rs`: successful check and
  build, then a nonzero or signal termination with stdout exactly
  `slice-contract-runtime:before\n`. The `after` marker must never appear.

The runner does not match panic text or demand a particular failure code.
Compilation failure, inability to launch, and timeout are failures, never proof
of a runtime bounds trap. The runner checks byte-exact stdout and retains stderr.

## Stock oracle now

From the repository root, with Python 3 and stock rustc on PATH:

```sh
python3 experiments/slice-contract-runtime.py \
  --output-dir "$JCODE_SCRATCH_DIR/slice-contract-stock"
```

`--rustc /absolute/path/to/rustc` selects a different stock oracle. Each source
is checked with `--emit=metadata` and separately built, then the fresh executable
is run. Explicit crate names allow the hyphenated fixture filenames. There are
no third-party Python dependencies. Every command has a 60-second timeout.
Default operation never invokes a native compiler and reports native `not_run`.

## Native gate after integration

Only after the slice implementation is integrated, explicitly opt in:

```sh
python3 experiments/slice-contract-runtime.py \
  --native /absolute/path/to/integrated/jr-rust \
  --output-dir "$JCODE_SCRATCH_DIR/slice-contract-native"
```

This also runs the stock oracle. The identical snapshot bytes are passed to
`jr-rust SOURCE --workers N --check` and
`jr-rust SOURCE --workers N -o NEW_EXECUTABLE`, for both N=1 and N=8.
If the binary's public `--help` advertises `--trace`, each check/build receives a
fresh trace path. Raw trace content and hashes are retained without assuming a
stable debug schema or inferring graph correctness from text. No native result
is claimed merely because this runner exists.

## Evidence and isolation

`--output-dir` is required. `evidence.json` records source SHA-256 hashes and
snapshot locations, commands/cwds, return codes, statuses, elapsed nanoseconds,
expected and observed stdout, complete stderr, lossless base64 streams, `rustc
-Vv`, tool paths and hashes, and native traces when advertised. Repository HEAD
is labelled checkout revision, not binary provenance. Native binary revision
is unknown unless independently established. A rustup proxy retains its invoked
basename, while the resolved target and hash are recorded separately.

Every invocation creates a unique mode-0700 run directory, with fresh outputs
and source snapshots retained for inspection. The current evidence is atomically
replaced with `incomplete` before running anything and updated during the run.
A failed rerun therefore cannot leave an older successful report masquerading
as current. Interruptions can leave `incomplete` or `running` evidence, neither
of which is a pass. Retained older run directories are historical, not current
results. The script exits zero only when all requested cases pass.

Elapsed times are observations, not benchmark evidence. This runner does not
validate native graph semantics, prove the emitted trap precedes a load, test
unsupported syntax, or replace the full integration regression suite.
