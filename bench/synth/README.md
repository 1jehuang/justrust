# Synthetic agent workload

One benchmark that covers what agents actually do, instead of a handful of
hand-picked edits. Three parts:

| file | what |
|---|---|
| `mine.py` | Mines Jcode sessions: every foreground cargo/justrust call in a Rust repo, the edits the agent made since its previous build, and the wall time. Writes `weights.json` (aggregates only, no paths or code). |
| `gen.py` | Samples points from `weights.json` in proportion to real call counts and turns each into a concrete command and concrete edits on the pinned suite snapshots. |
| `run.py` | Runs the points on one or more stacks, measured like System 3 (wall, CPU, peak RSS, user instructions of the build tree). |

```sh
bench/synth/mine.py                         # refresh weights.json from ~/.jcode/sessions
bench/synth/gen.py show --seed 1 -n 40      # print the plan (probes crates the first time)
bench/versus.py run --synth 1 -n 1          # System 3: stock vs jr-nocache vs justrust on 40 points
bench/versus.py run --synth 1 --stack x:K=V # plus a candidate stack
bench/suite.py run --synth 1 -n 1           # one stack (justrust defaults): commit to commit
bench/suite.py compare A.json B.json        # same seed: mean wall and instructions A -> B
```

## The axes

`mine.py` classifies each call:

| axis | values |
|---|---|
| command | `check`, `clippy`, `build`, `build-release`, `test` (no filter), `test-filtered`, `test-workspace`, plus the `-p` package and `--lib`/`--tests`/... flags |
| change | what was edited since the previous build call: `body`, `test`, `item` (a changed fn/struct/use/impl line), `new-file`, `manifest`, `vcs` (git checkout/pull/stash...), `type-error` (the build failed to compile, whatever the edit), or nothing: `rerun` (same command again), `switch` (another command on the same sources), `first` (session start) |
| spread | `one-file`, `multi-file`, `multi-crate`, `unknown` (git or a script that names no file) |
| relation | where the edits are relative to the built package: `target`, `upstream`, `outside` (a sibling repo), `unknown` |

Edits come from the `edit`/`write`/`multiedit`/`replace`/`apply_patch`
tools (also inside `batch`) and from bash commands that change sources
(`sed -i`, Python scripts that rewrite `.rs` files, heredocs, `cargo add`,
git operations).

## From a point to a run

`gen.py` maps the package onto the snapshot workspaces (Jcode Desktop with
its sibling Jcode, and justrust) and places the edits:

- **Live locations only.** Once per crate it inserts a type error after up
  to 300 `fn ... {` lines and runs `cargo check` (`--lib`/`--bins`, then
  `--tests`). A location is live if rustc reports E0308 for its probe:
  compiled code, the right cfg, not inside a macro. Lines live only with
  `--tests` are test code. Cached in `~/.justrust/bench/synth/probe/`.
- **Edits** (with a unique number per run, so rustc never sees a state
  twice): body and test edits insert `::core::hint::black_box(N)` at a live
  location, type errors insert `let _: u32 = "..."`, item edits add a pub
  fn to a live file, new files add a module, manifest edits add a feature.
- **Crates** for the edits are drawn by how often agents edited each crate
  of that repo, within the build's dependency closure (`upstream`: the
  target's local dependencies).
- **Test filters** are the edited file's module, else a module of the
  crate that has tests.

Per point and stack, `run.py`:

1. restores the sources and runs the command without running tests
   (unmeasured): the state just before the agent's edit,
2. applies the edits; for `switch`, runs the previous command (unmeasured),
3. runs the command, measured,
4. restores the sources.

The mean wall over points is the expected wait per agent call: points are
drawn in proportion to real calls, so no extra weighting is needed. The
report breaks it down by command and by change.

## Comparability

Same seed, same `-n`/`--points`, same generator fingerprint (hash of
`gen.py` and `weights.json`) and same snapshots means the same work.
Refreshing `weights.json` or changing `gen.py` starts a new series:
re-run the baseline.

## Known gaps

- About 13% of real calls happened in repos without a snapshot (the
  website, small side repos); they are not sampled.
- `rerun` measures the second identical build, which for tests is mostly
  test execution. Real reruns include flaky-test retries with other
  filters.
- Edit content is synthetic: a body edit is one statement, not a real
  change of the same size. Real item edits also change callers.
- Cold states (fresh worktree, branch switch to a far commit) come from
  `vcs` edits of up to six files, not real far-apart commits.
