# Agent instructions: justrust clean-room compiler (`main`)

This checkout is `main`: a from-scratch Rust compiler built as one
demand-driven graph. The shipping justrust client lives on the `stable` branch
in `~/justrust`. Client fixes go there, never here. Never merge `stable` and
`main`, and never tag releases from `main`.

## The job

**Make more real Rust programs compile and run correctly.** That is the only
measure of progress. Every session should end with compiler or library code
that makes a previously failing program or feature work, committed and pushed.

Progress = programs passing, not documents, inventories, audits, evidence
tooling, proposals, receipts or test-accounting systems. If a task does not
change what the compiler can compile, it is not progress. Do not do it unless
the user explicitly asks.

### Priority order for choosing work

1. A `fail` line in `tests/fixtures/full_programs/manifest.tsv`. Make it pass.
2. When all pass, add harder realistic programs (std-only, stock output in
   `<name>.stdout`, verify with `scripts/full_programs_oracle.sh`) and make
   them pass. Grow toward real crates: iterator chains, generics with traits,
   collections, string formatting, `?`/errors, closures, `Rc`/`RefCell`,
   modules, then multi-file crates, then unchanged crates.io crates
   (`experiments/adler2_repo_probe.py` is the first).
3. Fix a wrong-output (miscompile) or soundness bug. These outrank new features.
4. Implement the general form of the missing feature behind the failure, not
   the narrowest slice that satisfies one test.

Pick the failure that unblocks the most programs. Implement the whole feature
end to end (parse, resolve, type check, borrow check, lower, codegen, run).

## Do not do

- No new Markdown docs, progress reports, audits, findings, receipts, handoff
  manifests or "evidence" files. No updates to docs for routine progress.
- No compatibility inventories, rule-by-rule Reference reconciliation, pinned
  document acquisition, witness/obligation tracking, coverage percentages or
  Python accounting tools. These were tried (Oct 30 to Nov 6) and stalled all
  compiler progress for a week. They were deleted.
- No "authenticated", "source-bound", "sealed" or "readonly delivery" patch
  ceremonies. No hashing logs, no handoff JSON. Commit the change.
- No test-only or expectation-only commits that do not change compiler behavior,
  unless fixing a test that is genuinely wrong.
- Do not stall on blockers. If builds are temporarily unavailable, keep
  writing the next feature and validate when builds return. Do not fill the
  gap with meta work or by polling the blocker.

## Tests (keep them light)

For a language change, add cases to the nearest existing test file:

- valid programs: native CHECK + BUILD + run matches stock rustc output
  (workers 1 and 8 come for free from the existing helpers in `tests/support/`),
- a couple of invalid programs that must be rejected (even when unused),
- unsupported constructs must produce an explicit diagnostic, never a wrong
  executable.

That is enough. Do not create a new test file, doc and tool per small fix.

Before pushing:

```sh
cargo fmt
justrust test --test graph_integrity
justrust test --test full_programs
justrust test --no-run          # every test target must still compile
justrust test --test <the suites you touched>
```

`justrust test --lib` if you changed shared types. Never push red `main`. If a
pre-existing failure is unrelated to you, note it in the commit message and
move on. Do not stop to investigate it.

## Graph rules (the only hard architectural constraint)

The compiler is the graph (`docs/compiler/GRAPH-INTEGRITY.md`,
`tests/graph_integrity.rs`). Every compiler fact is a typed graph output read
through engine handles.

- Key nodes per file, per item or per generic instance. No new parameterless
  (whole-program) node kinds besides `check` and `build`.
- Request all independent prerequisites before awaiting any. Never block a
  worker thread.
- Do not read every item in a loop (a disguised whole-program join).
- Facts are immutable: no `Mutex`, `OnceLock`, atomics, lazy statics, global
  state or shared budgets in facts or compiler code. Local `Cell`/`RefCell`
  inside one node's computation is fine.
- Publish each output as soon as it is final. No placeholders.
- Identities are canonical: resolved declaration IDs and ordered type
  arguments, never display names.
- `tests/graph_integrity.rs` baselines may only shrink. Never raise one without
  explicit user approval.

Known debt: `resolve` and `signatures` are still whole-program nodes. Split
them when nearby. Do not copy the pattern.

Engine API (`src/engine.rs`): `cx.request(key)`, `cx.read_fact::<T>(handle)`,
`cx.publish_fact(i, v)`, `cx.fork(fut)`, `cx.fail`. Node kinds are the
`const ...: u32` values in `src/compiler/mod.rs`; `docs/compiler/COMPILER.md`
describes each one.

## Correctness contract

Edition 2024, Linux x86-64, oracle in `tests/support/stock_rustc.rs`. Valid
supported programs behave exactly as stock rustc. Invalid programs are rejected.
Unsupported constructs get an explicit diagnostic, never an approximate
translation. CHECK must not demand codegen. Do not link, call or ship rustc,
Cargo or LLVM as part of the compiler. Std-library Rust source in `libraries/`
is compiled by our compiler.

### Using the vendored toolchain source

The vendored Rust toolchain in the stable checkout
(`~/justrust/toolchain/rustc`: `compiler/`, `library/`) may be used to speed up
development:

- Read it freely to learn exact language rules and algorithms (method lookup,
  autoderef, coercions, inference defaults, trait selection, closure capture,
  borrow checking). Implement them in our own graph-shaped code. rustc's
  whole-program inference context does not fit the graph rules, so port the
  rule, not the data structures.
- Std library code (`library/core`, `library/alloc`, `library/std`) may be
  copied or adapted into `libraries/`. Keep the upstream license: add a header
  comment on each adapted item or file naming the source path and
  "MIT OR Apache-2.0, Rust Project Contributors" (license texts are in
  `libraries/rust-upstream-licenses/`). The `LICENSING.md` row covers every
  file carrying that header. Only port what our compiler can actually compile; a port
  that needs an unsupported feature must still fail with an explicit
  diagnostic.
- Compiler code (`compiler/rustc_*`) may be adapted under the same attribution
  rule when it fits the graph rules. Graph integrity rules still apply to
  everything.

## Working in the shared checkout

Several agents share `~/justrust-clean-room` on `main`. No branches, no
worktrees.

- Edit files in place. Prefer new modules over large edits to the big shared
  files (`frontend.rs`, `ir.rs`, `type_check.rs`, `cfg.rs`, `ownership.rs`,
  `traits.rs`, `backend.rs`).
- Commit only your own hunks: `git add -p` or `git add <your files>`. Do not
  `git add -A`, stash, reset or rebase over others' uncommitted work.
- Commit and push each working improvement right away with a concise message.
  If the push is rejected, `git pull --rebase --autostash` and push again.
  Use the configured Git identity.

## Swarm coordination

The coordinator assigns **features or failing programs**, never inventories,
audits or evidence tasks. Workers edit the shared checkout directly and commit
their own work. Do not require workers to build in scratch archives and hand
back sealed patches for the coordinator to integrate: that serializes the whole
swarm through one agent. If two workers need the same file, split by function
or sequence them.

## Builds

Use the installed stable `justrust` client (`justrust check`, `justrust test`,
`justrust clippy`) and `cargo fmt`. Do not install this package over the stable
client. Follow the machine's configured build policy (the cloud VM forces
hosted builds).

## Where things are

| Need | Read |
|---|---|
| What each node kind does, supported syntax | docs/compiler/COMPILER.md |
| Graph structural rules and current debt | docs/compiler/GRAPH-INTEGRITY.md |
| Graph design, engine semantics | docs/compiler/ARCHITECTURE.md 17-18, ENGINE.md, PROTOTYPE.md |
| Target program corpus | docs/compiler/FULL-PROGRAMS.md |
| Remaining feature areas | docs/compiler/IMPLEMENTATION.md (G1-G8) |
| Optimizer and codegen direction | docs/compiler/OPTIMIZATION.md |
| A specific feature's design | the matching file in docs/compiler/ |

Feature docs describe design. Read them when relevant. Do not write new ones.
Historical material in `bench/`, `data/runs/`, `experiments/results/` and
CHANGELOG.md is reference only, not instructions.

Preserve licensing and attribution (LICENSE, NOTICE, LICENSING.md,
THIRD-PARTY-LICENSES.md).
