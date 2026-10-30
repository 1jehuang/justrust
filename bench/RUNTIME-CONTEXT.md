# Opt-in benchmark application runtime context

`JUSTRUST_BENCH_RUNTIME_CONTEXT=jcode-isolated-v1` selects an experimental
runtime policy for commands executed by `versus.run_counted`, including
`bench/versus.py` and the synthetic runners used by both comparison modes.
Unset preserves inherited runtime semantics and the legacy counted-result shape.
Unknown values fail before command execution.

Use the **same selector for both complete runs** and leave every workload point,
command, test argument, source pin, compiler identity and query flag unchanged:

```sh
JUSTRUST_BENCH_RUNTIME_CONTEXT=jcode-isolated-v1 bench/suite.py run --synth 1 -n 1
JUSTRUST_BENCH_RUNTIME_CONTEXT=jcode-isolated-v1 bench/versus.py run --synth 1 -n 1
```

These are configuration examples, not authorization to restart a stopped suite.
The October 30 retention experiment still requires coordinator approval before
any broad run. Existing partial results with inherited context are not a baseline
for an isolated-context candidate.

Each counted command gets a fresh private HOME, JCODE_HOME and XDG config,
data, state, cache and runtime directories. Inherited JCODE, Anthropic, OpenAI,
OpenRouter, Claude, Codex, Gemini, Google API and Git environment fields are
removed. This deliberately represents an unauthenticated app, not an account with
invented credentials. Existing explicit CARGO_HOME and RUSTUP_HOME survive,
otherwise they are resolved against the original HOME before isolation. Compiler
and justrust flags, identities, private toolchain prefixes and cache controls
are not rewritten. This is application isolation, not a sandbox that denies all
access to host credentials or arbitrary paths.

For archived sources with no ancestor `.git`, the policy creates an empty Git
metadata directory outside the source tree and an exclusive temporary `.git`
pointer at the command's working directory. This supports both Git itself and
application filesystem discovery. It does not create another checkout/worktree,
commit, index, tracked source change, or replacement snapshot. Existing Git
metadata is preserved. Normal return or Python exception removes owned metadata
and runtime state. Concurrent runs against the same archive are unsupported.
Uncatchable termination can leave a pointer behind, so after a killed run inspect
and remove only the pointer owned by that interrupted context before restarting.
Do not delete preexisting repository metadata.

Result environment metadata and isolated counted records include a versioned,
path-free policy description. `suite.py compare` and `versus.py compare` reject
mixed runtime policies, including legacy inherited results versus isolated ones.
Context setup/teardown is outside the counted command timer, equally for stock
and justrust. No speed claim is implied.

## Frozen-binary investigation, October 30, 2026

No builds, source/test changes, assertion changes, skips, snapshot repins or
workload removal were used. The eight originally failing test names were each
run as `<unchanged binary> <full-test-name> --exact --nocapture`, first with
inherited context, then private runtime state, then the reusable policy.
Full output, command reconstruction script, test names, exit codes and binary
SHA256 values are retained locally at:

```text
/home/jeremy/.jcode/scratch/bench-hermeticity-proof.py
/home/jeremy/.jcode/scratch/bench-hermeticity-proof-20261030/results.json
/home/jeremy/.jcode/scratch/bench-hermeticity-proof-20261030/context-v1-results.json
```

The inherited Anthropic catalog assertion fails, but passes with private app
state. Both recent-project-review tests fail without Git metadata and pass with
the policy's temporary Git context. Four other original TUI failures pass when
run individually in either inherited or isolated context, so their whole-suite
ordering/global-state behavior is **not validated** by this focused replay.

`answering_no_on_continue_prompt_lands_on_regular_session` still fails in both
contexts. The pinned implementation returns a login suggestion when no auth is
available and considers that suggestion an active welcome screen, whereas the
test asserts empty suggestions or an inactive welcome screen. Runtime isolation
must not fabricate authentication to hide this remaining correctness issue.
The coordinator must resolve it outside the harness-only lane before restarting
a complete comparison. All original failure logs remain retained.

Focused harness regression command:

```sh
PYTHONPATH=bench python3 -m unittest -v test_runtime_context test_synth_context test_run_counted test_versus test_suite
```
