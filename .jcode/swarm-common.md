# Common rules for cricket's swarm workers (justrust clean-room compiler)

Goal: full Rust support in the native graph-engine compiler. First priority is
compiling complete, realistic single-file programs end to end (stock rustc
output == native output).

Repository: /home/jeremy/justrust-clean-room, branch `clean-room-rewrite`
(remote origin). Read AGENTS.md (top clean-room section), GRAPH-INTEGRITY.md,
RUST-COMPATIBILITY.md, IMPLEMENTATION.md and COMPILER.md first.

Isolation: many agents edit the big shared files (frontend.rs, ir.rs, cfg.rs,
ownership.rs, borrow.rs, backend.rs) at once. Do NOT edit in the main checkout.
Create your own worktree:
  git -C /home/jeremy/justrust-clean-room fetch origin
  git -C /home/jeremy/justrust-clean-room worktree add --detach ~/.jcode/scratch/cr-<yourlabel> origin/clean-room-rewrite
Work there. Integrate often: commit small, `git fetch && git rebase origin/clean-room-rewrite`,
re-run the relevant tests, then `git push origin HEAD:clean-room-rewrite`. Resolve
conflicts by preserving other agents' work. Never force-push. Never rewrite others' commits.
Use the user's git identity (already configured). Push every working increment.

Ownership: src/engine.rs, src/compiler/mod.rs and src/compiler/instances.rs belong to
agent `bat` (graph slot-to-fact conversion) until it announces completion. Don't
edit them. If your feature needs a change there, make it minimal and DM cricket first.

Hard rules:
- `justrust test --test graph_integrity` must pass. Never raise a ratchet or add
  baseline lines, OnceLock/Mutex/static state or whole-program node kinds.
- Use `justrust check/test/clippy` (not cargo), `cargo fmt` before commit, keep clippy clean.
- Use stock rustc (installed) only as an external oracle (edition 2024, -C panic=abort).
  Never call it from the compiler.
- Valid supported programs must produce correct output, invalid programs must be
  rejected (even unused), and unsupported programs must fail with an explicit diagnostic.
  Never approximate.
- Prefer general, real implementations over narrow "bounded slice" special cases.
  The aim is breadth toward real programs. Keep correctness, but don't spend hours on
  ceremony documents. A short `<FEATURE>.md` plus tests and a line in IMPLEMENTATION.md is enough.
- Full-suite regressions: run `justrust test` before pushing (hosted is fine).
- Report progress to cricket with swarm dm (to_session cricket) when you push a
  milestone or are blocked. When finished, give a final report with commits.
