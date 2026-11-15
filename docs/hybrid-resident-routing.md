# Experimental hybrid resident routing

The hybrid experiment chooses a ready owned resident worker, or the existing
justrust Cargo/hosted path when native execution has not begun. It is not enabled
by default. The engine remains explicitly Blaze-derived, with Cranelift and its
own pinned toolchain. This is not a switch that keeps shipping rustc's heap alive.

## Explicit use

```sh
export JUSTRUST_NATIVE_ENGINE=/absolute/path/to/owned/jr-engine
export JUSTRUST_HYBRID_TOOLCHAIN=/absolute/path/to/its/pinned/toolchain
# Optional: an existing short canonical directory, owned by you with mode0700.
export JR_ENGINE_HOME=/absolute/private/hybrid-home
justrust hybrid test --lib -- test_filter
justrust hybrid prewarm test --lib -- test_filter
justrust hybrid test --lib -- test_filter
justrust hybrid status
justrust hybrid stop
```

Without an explicit home, justrust creates a private `JUSTRUST_HOME/hybrid`
directory. Native targets and intermediate build directories are separate from
the caller's Cargo artifacts. The matching toolchain is applied only to the
native child. The fallback retains the original compiler, target directory,
arguments, environment, working directory and ordinary routing policy.

`JUSTRUST_HYBRID=ready` explicitly opts ordinary `justrust build`, `test`, `check`
and `run` into the same experiment. Other commands and the `cargo` proxy are
unchanged. Remove that variable to disable it. A missing default engine or
missing native toolchain setting selects the normal path without native work.
Explicit invalid executable/toolchain paths or unsafe state directories produce
errors instead of being silently repurposed.

## Deferred preparation, separately opt-in

`JUSTRUST_HYBRID_PREWARM=after-success` additionally requests
background preparation **after** a successful Cargo fallback has completed.
Without it, preparation is manual. Failed fallback commands do not trigger it.
Only one preparation runs at a time in a hybrid namespace, protected by an
OS-held lock rather than a persistent PID or lockfile-existence guess. Its
output goes to the private home's `prewarm.log`. A preparation-launch failure
does not change the already-completed foreground result.

The internal supervisor has its own session and preserves process priority.
Lowering its nice priority would also permanently lower newly started resident
workers, including later foreground edits. It waits for a
compile-only native preparation operation and holds the deduplication lock
until that operation finishes. Prewarming must never execute user tests or
`run` programs. Build scripts and proc macros are compile-time code, so
preparation is not side-effect-free or a sandbox. A later foreground request
can still contend with background preparation. That interference needs actual
measurement before automatic preparation can become a default.

Hosted job ownership and cleanup remain authoritative. This local dispatcher
does not bypass account admission or make a background process durable across
an owning hosted job's cleanup. Production hosted deployment requires validating
that lifecycle, packaging and tenant isolation, not just running the binaries
inside an authenticated guest shell.

## Exactly-once routing protocol

1. The client launches `jr-engine cargo hybrid-ready -- COMMAND ARGS` with a
   private inherited decision descriptor `JR_HYBRID_DECISION_FD`.
2. Only exact bytes `miss\n`, a closed decision descriptor and successful engine
   exit authorize Cargo fallback. This must occur before native compilation or
   user-program/test execution. A cold probe must not start a daemon or download
   a toolchain.
3. Atomic native readiness admission precedes `committed\n`. The descriptor is
   closed before compilation/execution or spawning descendants. Once committed,
   all diagnostics, failures, status125 and termination signals are terminal.
   They must never trigger a Cargo retry or execute the test/program twice.
4. Missing, malformed or timed-out decisions are errors, not fallback signals.
   The client bounds decision reads and refuses ambiguous retries.

A readiness check followed by an ordinary cold-starting native invocation is
not sufficient: eviction or a concurrent build can happen in between. The
engine owns readiness admission and the dispatch reservation.

## Promotion requirements

An opt-in implementation passing mock/subprocess routing tests is not proof of
native Rust correctness or a speedup. Promotion requires actual cold/warm,
eviction, dependency/configuration/proc-macro/build-script changes, errors and
recovery, exactly-once side effects, separate artifact directories, foreground
interference, memory and cleanup checks. Broad default promotion also requires
the canonical session-weighted suite before/after comparison with matching
seed, generator, weights, snapshots and execution mode, plus valid full test
contracts and command/change regression breakdowns. Warm setup cost and native
coverage must be reported, not hidden by a run that always falls back.

Do not infer a universal one-second worst case from the earlier focused fixture.
Unchanged shipping fallback still has routing overhead, and committed native
builds can encounter their own slow paths or unsupported code. Complete decoded
compiler-state disk snapshots are a different, still unmeasured design.
