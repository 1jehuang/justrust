# 2026 in the Rust compiler, and what ra-jit does about each part

2026-10-20. Source: panstromek, a member of rustc's performance triage,
[2026 is an exciting year for the Rust compiler (and me)](https://yoyo-code.com/2026-is-exciting-year-for-rust-compiler-and-me/),
dated 19 October 2026, read in full. Each claim is marked:

- **read**: in the post, or in source ra-jit or rustc publish, which we read;
- **reported**: the post says so and we haven't checked it;
- **unverified**: our inference.

What ra-jit is, for the mapping: rust-analyzer's front end (salsa, the next trait solver)
lowering to MIR, compiled by Cranelift (default) or LLVM, with the first build and the
dependencies built by rustc through cargo.

## Next trait solver

- The next-generation trait solver is close to done, after almost 10 years; it fixes soundness
  bugs and unblocks async and `impl Trait` work (reported).
- ra-jit already infers with it: rust-analyzer's `hir-ty` runs `ra-ap-rustc_next_trait_solver`
  0.166 (read, `Cargo.toml`). When rustc switches, rustc and ra-jit agree on more programs,
  not fewer (unverified).
- **ra-jit:** follow the `ra-ap-rustc_*` releases each time the fork is rebased; nothing to build.

## Polonius

- A new borrow checker, enabling more code to compile (reported).
- ra-jit has no borrow checker; a program rustc would reject can run (read, `docs/support.md`).
  Polonius doesn't change that, but it moves what rustc accepts, and so what a ra-jit user is
  told by the `cargo check` they keep running (unverified).
- **ra-jit:** production gap 3 in [mir-gap.md](mir-gap.md#production-gaps).

## Parallel front end

- Close to stabilization, driven by @petrochenkov (reported).
- ra-jit's front end is rust-analyzer's, already query-based; the first build and dependencies
  are rustc's, so they get faster for free once it's the default (unverified).
- **ra-jit:** nothing to do; pass `-Zthreads` through untouched if users set it.

## LLVM 23

- LLVM 23 made rustc unambiguously faster, from new optimizations and LLVM getting faster
  itself (reported).
- rustc 1.99.0, which ra-jit pins, ships LLVM 23.1.1 (read, `rustc -vV`); ra-jit's LLVM tier
  builds against LLVM 20 (read, `docs/support.md`, `docs/results.md`).
- **ra-jit:** move the LLVM tier to LLVM 23. That's the tiers work's area (`llvm/lower.rs`,
  `tier.rs`), so it's a recommendation here, not a change.

## Many small performance wins, and triage

- Steady compile-time improvements, more contributors, regressions triaged by the people who
  merge (reported).
- **ra-jit:** the same habit fits: the benchmark dashboard and the edit-latency harness gate
  merges. Owned by the benchmark work; nothing new here.

## Move elimination

- A MIR pass, proposed by @Amanieu, removing the copies LLVM can't, which speeds compile time and
  run time; Rust emits more moves than C++ (reported). It changes MIR's semantics a little
  (reported).
- On rustc's master (2026-10-20), `TailCopyToMove` turns a final `copy` before a return into a
  `move` at level 2 with `-Zmir-move-elimination`; its doc comment says it exists so the
  coming `MoveElimination` can merge the returned local with `_0`. `MoveElimination` itself is
  not in the pipeline yet (read, `docs/upstream/rustc-mir-transform/master/`).
- This matters more for ra-jit than for rustc: Cranelift does no memcpy elimination, so every
  MIR copy of an aggregate is a copy in the machine code (unverified, from Cranelift's design).
- **ra-jit:** done here, as a first step: rustc's copy propagation and dead-store elimination,
  ported (`RA_JIT_MIR_OPT`). Next: port `TailCopyToMove` and `MoveElimination` when they land.

## Polymorphization

- A third attempt, Swift-style: generic code compiled once with layouts passed at run time, a
  flag-controlled runtime cost, driven by @camelid (reported).
- ra-jit monomorphizes every instance it compiles (read, `monomorphized_*_query` in
  `hir-ty/src/mir/monomorphization.rs`). A generic function edited in a hot loop is recompiled
  once per instance; Swift-style polymorphization would make that once (unverified).
- **ra-jit:** worth an experiment for the edit loop only, once rustc's design settles; track
  the goal.

## `cargo check` and `cargo build` sharing a cache

- A goal by @blyxyas that would let check and build share artifacts (reported).
- **ra-jit:** its own cache (`docs/cache.md`) keys on its compiled bodies, not rustc's; when
  cargo shares check and build artifacts, ra-jit's first build could start from a `cargo
  check`'s metadata (unverified).

## `-Zno-embed-metadata` by default

- Shrinks the target directory 5-30% (reported; announced on the Inside Rust blog, 2026-08-18).
- ra-jit reads metadata from the `.rmeta` cargo writes, and skips `lib.rmeta` inside an rlib
  when it links (read, `cargo/wrapper.rs`, `symbols.rs`); an rlib without it is fine
  (unverified, no test yet).
- **ra-jit:** add a test with `-Zembed-metadata=no`.

## The new `build-dir` layout

- `build-dir` layout v2, in testing since 2026-03-13; it enables finer locking and caching
  across workspaces (reported).
- ra-jit reads cargo's present layout: `<profile>/deps`, `<profile>/build` and
  `.fingerprint` (read, `deps.rs`, `cargo/state.rs`), and nothing in it reads `build-dir`
  (read, no match for it in `crates/ra-jit`). With a custom `build-dir`, or once v2 is the
  default, ra-jit won't find dependency artifacts (unverified, not tried).
- **ra-jit:** production gap 2 in [mir-gap.md](mir-gap.md#production-gaps).

## `-Zno-link` and `-Zlink-only`

- @bjorn3 is fixing them, so build scripts and proc macros pipeline (reported).
- ra-jit already splits compiling from linking itself, through its cargo wrapper (read,
  `cargo/plan.rs`). Nothing to do; build scripts get faster under it for free (unverified).

## Architecture: attributes, queries, expansion-time evaluation

- Attribute parsing unified, the query system cleaned up, groundwork for evaluating at
  expansion time, const generics progressing (reported).
- **ra-jit:** none of it is code ra-jit shares; rust-analyzer follows on its own schedule.

## Project goals and funding

- Project goals have settled; there's a Fast Builds roadmap, a Maintainers in Residence
  program and a funding team (reported).
- **ra-jit:** the Fast Builds roadmap is the place to show a JIT edit loop, once ra-jit's MIR
  matches rustc's closely enough to make the comparison fair (unverified).
