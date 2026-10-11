# Recorded runs

One slim JSON line per recorded cargo run, written by `justrust export`
(see `src/export.rs`). Raw runs stay in `~/.justrust/runs/` (hundreds of MB
with full compiler output). This directory keeps what findings and replay
benchmarks need: wall time, phases, slowest units and passes, rebuild
reasons, tests, CPU, memory, and contention.

```text
data/runs/public-repos.txt          repos allowed to keep details
data/runs/<machine>/<YYYY-MM>.jsonl one line per run, sorted by id
```

Fields added by the export: `machine`, `repo`, and `kind`:

- `agent`: an agent session in a real checkout (the workload to optimize)
- `manual`: a real checkout, no agent session
- `bench`: justrust benchmark copies (`~/.justrust/bench`)
- `scratch`: throwaway copies and temp dirs

This repository is public. Paths under the home directory become `~/...`, and
first errors keep one line. Runs from repos not listed in `public-repos.txt`
keep only numbers: `repo` is `private`, unit names become `crate0..`, errors
keep only their code, and paths, arguments, and rebuild reasons are removed.

Update with `justrust export` from the repo root, then commit. It only
appends runs that are not exported yet.
