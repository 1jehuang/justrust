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

## Retention policy

Executable regression tests and current automated results are the source of truth
for correctness and progress. Routine successful check/test/lint runs do not need
an export, a committed log, or a documentation update. Rerun the relevant suites
when needed; keep their normal test-runner/CI output tied to the tested revision.
Existing files here are historical records, not a requirement to keep adding runs.

Retain selected summaries only when they support a before/after performance claim
or preserve a hard-to-reproduce failure. Include the revision, inputs and command
needed to reproduce the result, and prefer permanent regression tests for bugs.
For selective retention, run `justrust export --out <scratch-directory>` from the
repo root, then copy only the relevant redacted summaries here. Do not commit the
whole rolling monthly archive merely because tests ran. Automatic local recording
can remain enabled independently of this export policy.
