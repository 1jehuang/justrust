# Full programs: the target corpus

"Compile a full program" means: every program in
`tests/fixtures/full_programs/` is native-checked, built and run, and prints
exactly what stock rustc 1.98.1 (`--edition 2024 -C panic=abort`) prints.
That is the first-priority acceptance gate. Passing it is still a subset
milestone, not full Rust support (see RUST-COMPATIBILITY.md).

- 20 realistic single-file programs, std only, increasing difficulty
  (hello, fizzbuzz, collatz, sieve, gcd/fib, matrix, bank account, enum state
  machine, generic stack, string processing, closures/iterators, linked list,
  trait objects, generic containers, `Result`/`?`, sorting, HashMap/BTreeMap,
  tokenizer+calculator, operator overloading, Rc/RefCell inventory).
- `<name>.stdout` is the stock oracle output. Run `scripts/full_programs_oracle.sh`
  to verify it, or `--write` to regenerate. Stock is only an oracle. The test never runs it.
- `manifest.tsv` holds the ratchet: `pass` programs must keep passing. `fail` programs
  must be rejected with an explicit diagnostic (a wrong-output build is a
  miscompile and fails). An unexpected pass fails until you flip the line.
  The `needs` column lists missing feature tags. When a feature lands, delete its tag
  everywhere. A `pass` line must have empty needs.
- `justrust test --test full_programs -- --nocapture` prints per-program
  `PASS` or the first native diagnostic.

## Unchanged Adler2 repository probe

Run the pinned Adler2 2.0.1 target without adapting its source. The probe
requires Python 3.11 or newer for standard-library `tomllib`; on hosts where
`python3` is older, use `python3.11` explicitly instead.

```sh
python3 experiments/adler2_repo_probe.py \
  --native ./target/debug/jr-rust \
  --output-dir "$JCODE_SCRATCH_DIR/adler2-probe-new"
```

The default input is an already-cached Cargo registry copy of Adler2 2.0.1.
Alternatively pass `--repo /path/to/adler2`; both upstream Rust files must match
its reviewed SHA-256 pins (release provenance revision
`89a031a0f42eeff31c70dc598b398cbf31f1680f`). No download happens. The runner
copies the package and supplied native binary into the new output directory;
it never edits the input repository or reuses an old output directory.

The stock oracle builds/checks the unchanged library in `no_std` and default
`std` configurations, runs upstream unit tests, and runs a separate consumer
against five checksum vectors, including incremental updates. Native workers
1 and 8 attempt the real project check/build/test interfaces and the separate
path-dependent consumer. A direct `--lib --check` attempt also captures an
independent source diagnostic. That diagnostic is not library-build success.

`report.json` records exact commands, binary/source hashes, per-stage statuses,
full diagnostics, compatibility blockers and missing-dependency blockers.
Per-command stdout/stderr files retain the raw output. Exit 0 requires every
required native stage to pass, exit 1 means compiler compatibility is blocked,
and exit 2 means an input/environment/oracle problem needs attention. A run
can expose compatibility blockers even while its overall result is
`environment_blocked`. Never treat timeouts, missing artifacts, or a successful
source-only check as a successful target build.

This is a first-blocker probe, not a complete Rust gap inventory. Later errors
can remain hidden. Doctests, benchmarks and optional internal/docsrs features
are outside this target, and the stock baseline uses rustc directly rather
than claiming stock Cargo workflow coverage. The older
`adler2_adapted_acceptance.py` remains a different, explicitly adapted test.
Runner regression tests:

```sh
python3 -m unittest discover -s experiments -p test_adler2_repo_probe.py
```

## Growing the corpus

When every line passes, add the next harder realistic program rather than
polishing accounting. Good next targets: iterator-heavy data processing,
generic traits with associated types, `impl Trait` and closures returning
closures, `String` formatting with width/precision, multi-module programs,
`Box<dyn Error>` and `?` across error types, `BTreeMap`/`HashSet`/`VecDeque`,
small interpreters and parsers, then multi-file crates and unchanged crates.io
crates. Current pass/fail status is the manifest itself.
