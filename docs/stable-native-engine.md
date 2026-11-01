# Stable client: experimental native resident engine

## Scope and integration contract

The shipping client on `stable` remains Cargo-driven by default. Ordinary
`justrust build`, `test`, `check`, and `run` do not become resident-engine builds
because source has been imported. The stable integration is an
**experimental, explicit opt-in** command family:

```text
justrust native <build|test|check|run|status|stop|toolchain>
```

This is a stable-client entry to the owned experimental compiler, not a claim
of complete Rust compatibility. Build/test/check/run select the
resident path explicitly. Status and stop address its resident lifecycle, while
toolchain addresses its matching toolchain. An engine implementation and its
matching runtime/toolchain artifacts are separate from the shipping client.

`JUSTRUST_NATIVE_ENGINE` is the explicit override and must name an **absolute
binary path**. By default the dispatcher selects `jr-native` beside its own
executable, never an arbitrary executable from PATH. There is no automatic download or
selection of an upstream Blaze binary. A missing owned engine must be reported,
not silently replaced by a different compiler. The stable dispatcher never downloads
an engine. Runtime toolchain provisioning remains the engine's separately documented
behavior. The stable command must preserve engine
output and exit status, and must not silently turn unsupported native behavior
into a successful stock build. Stock Cargo remains a separate compatibility
path, with separate artifacts and protocols.

## Build and use the owned engine

Build from this checkout with the exact toolchain named by
`toolchain/resident/harness/toolchain` and its full compiler commit in
`crates/ra-jit/src/cargo/toolchain/pins.txt`:

```sh
toolchain/resident/build-jr.sh /absolute/path/to/pinned-toolchain
justrust native test --lib -- some_test_filter
justrust native status
justrust native stop
```

The helper uses justrust for a controlled local bootstrap build because the
native compiler requires that exact toolchain. It installs the source-identified
`jr-engine` binary as `jr-native` in `${CARGO_HOME:-$HOME/.cargo}/bin`, with the
Blaze-derived license/provenance notices alongside it. An optional second
argument selects another install directory; set the absolute engine override
when that is not beside the stable client. This does not replace `justrust`.
At runtime the engine still requires its matching sysroot: point `RUSTC` at that
toolchain's `bin/rustc` to reuse it explicitly. Otherwise the engine's existing
toolchain resolver can use a matching rustup installation or provision its own
matching Rust distribution. The helper itself never downloads an upstream
Blaze engine or substitutes the shipping compiler for the pinned bootstrap.

The client creates its default native home at canonical `JUSTRUST_HOME/native`
(default `~/.justrust/native`) with mode 0700. The engine validates ownership,
mode and canonical paths before initializing separate runtime and versioned
artifact stores. It does not reuse upstream Blaze stores and forces its remote
store synchronization off. An explicit `JR_ENGINE_HOME` must already be an
absolute canonical owner-only directory. Use a short path if your user-home
location exceeds Unix socket path limits. Existing unsafe homes are not chmodded
or silently repurposed. Real HOME and Cargo configuration remain visible, so
this is not a sandbox or a guarantee that all project writes stay in the home.

The native entry preserves arguments including `--`, inherited environment,
working directory, stdin/stdout/stderr, exit codes and signals. It replaces
the client process on Unix. It never reruns a failed native request with Cargo,
which could hide diagnostics or execute tests/programs twice. Missing engines
and unsupported top-level commands produce actionable errors before execution.
Non-UTF-8 arguments and non-Unix integration are explicitly unsupported. The
entry bypasses Cargo-specific default setup and account/update polling; native
requests are not presented as ordinary recorded Cargo/hosted builds.

## Stable-entry validation, 2026-11-01

The new stable entry passed five unit tests and 13 Unix subprocess tests for
exact argv/separators, inherited environment/cwd, output, exit 101, SIGTERM,
missing/relative/self engine selection, unsupported commands, help without
setup, default 0700 state, and unmodified explicit home overrides. The full
client invocation reported 866 passed and 12 ignored; all-target clippy with
`-D warnings` passed. Hosted checking was attempted but the build endpoint reset
the connection, so subsequent validation used the documented local-outage path.
The four cited unit/integration/full-test/lint summaries are in
`data/runs/stable-native-entry-20261101/2026-11.jsonl`.

After building the owned engine with the helper and reinstalling the stable
client, the installed `justrust native test` passed seven real pinned-project
phases: cold, noop, reachable test-body edit with a fresh runtime nonce, E0308,
type recovery, assertion failure, and assertion recovery. The selected four
tests and expected failure statuses were checked. This used default sibling
engine discovery and fresh `JUSTRUST_HOME/native` state, an explicitly matching
`RUSTC`, offline Cargo and disabled proc-macro servers. Original source bytes
were restored, native stop succeeded, and stores were not deleted. The
privacy-projected receipt is
`data/runs/stable-native-entry-20261101/native-acceptance.json`.

This is correctness/integration evidence, not a new comparative benchmark. It
does not establish Windows support, general Rust parity, or a broad speedup.
The dispatcher is a transitional CLI entry into the owned compiler, not a
reimplementation of Blaze's compiler techniques or a permanent substitute for
integrating native recording/scheduling and distribution.

## Owned source, provenance, and licenses

The resident implementation is ordinary owned source at `toolchain/resident/`,
not a submodule and not an independent clean-room compiler. It is explicitly
**Blaze-derived**. The import records upstream tag `v0.20261027.0` and exact commit
`5d4f78f7577cf7170993b74f735099dd03f5c6ca` in
`toolchain/resident/JUSTRUST-SOURCE.md`. That is a pinned provenance statement,
not a claim about the latest upstream release. Local development can diverge
without requiring upstream internal compatibility.

Package licensing is `MIT OR Apache-2.0` in
`toolchain/resident/Cargo.toml`. Preserve upstream authorship,
`toolchain/resident/LICENSE-MIT`, `toolchain/resident/LICENSE-APACHE`, and vendored
dependency notices. Importing licensed source does not establish correctness or
performance. Build identity must explicitly identify the derived engine rather
than accidentally reporting the enclosing justrust Git revision as upstream.
The source-import document records its import-time unbuilt/unvalidated status,
not a perpetual assertion about all later builds.

## Exact resident techniques

The architectural map is `toolchain/resident/docs/architecture.md`. Paths below
are relative to `toolchain/resident/` unless prefixed otherwise. Its historical
wrapper sections and newer whole-build sections describe different execution
paths and must not be conflated.

### Resident rust-analyzer and Salsa analysis

`crates/ra-jit/src/cargo/daemon.rs` dispatches requests to workers, while
`crates/ra-jit/src/main.rs` holds a rust-analyzer database in its long-running
worker mode. The daemon itself does not hold the program database. Worker inputs
include repository, toolchain, target, feature set, and whether libraries use
`cfg(test)`. Parsing, resolution, inference, MIR, layout, and constant evaluation
are query-driven rather than discarded after every compiler invocation.
`Cargo.toml` selects the owned `vendor/rust-analyzer/` and `vendor/salsa/` sources.

`crates/ra-jit/src/outline.rs` distinguishes body-only edits from changes visible
to other code. A `const fn` body, an async or opaque-return signature dependency,
a body passed to a macro, or nested item declarations can invalidate more than a
simple private body. `crates/ra-jit/src/inputs.rs` tracks manifests, lockfile,
configuration, environment and build-script inputs so a warm database is not
reused merely because an edited Rust file looks local.

The owned rust-analyzer body representation uses item-relative ranges
(`Body::relative` in `vendor/rust-analyzer/`). Moving surrounding source therefore
need not invalidate an unchanged function body. Code locations in
`crates/hir-ty-codegen/places.rs` are likewise relative to the item and adjusted
when emitting an object. This is the architectural basis for function-level
lowering and inference reuse, not a promise that every source edit touches only
one function.

### Function-level code generation and storage

`crates/hir-ty-codegen/fragment.rs` makes compilation of a monomorphized function
a database query. The result names referenced functions and data by identity,
not their eventual object-file positions, so Salsa can retain it independently
of final layout. `crates/hir-ty-codegen/home.rs` compiles generic instances from
their home crate's perspective rather than making them depend on each caller's
workspace context. Changed query results are stitched into objects.

`crates/hir-ty-codegen/backend.rs` defines the producer boundary: machine bytes,
relocations, and unwind information. **Cranelift is the default producer**. The
front end lowers monomorphized MIR into Cranelift IR with memory-backed locals.
Layout and call conventions use the pinned rustc ABI machinery, including owned
`vendor/rustc_abi/`. Vtables, TypeId calculation and unwinding must agree with
the matching runtime wherever code crosses compiler boundaries. This is a
compatibility obligation, not proof of universal ABI support.

Optional LLVM production translates each function's Cranelift IR separately
(`crates/hir-ty-codegen/llvm.rs` and `llvm/`) and can return unsupported lowering
to Cranelift with a recorded reason. There is no cross-function inlining in that
model. Wasm has a separate lowering and relocatable-object writer, not a
Cranelift WebAssembly backend.

`crates/ra-jit-store/` provides the content-addressed function store.
`crates/hir-ty-codegen/stable.rs` and `cached.rs` provide stable instance identities
and persisted function reuse. Owned-engine artifacts and incompatible toolchain
identities must not be confused with stock Cargo/rustc artifacts.

### Linking, patching, and execution

The historical Cargo-wrapper path captures rustc's link line and symbols in
`crates/ra-jit/src/cargo/wrapper.rs`, `units.rs`, and `plan.rs`. It can place
resident objects ahead of the matching rustc objects. Mach-O requires weakened
copies rather than assuming ELF duplicate-definition behavior. This is distinct
from native whole-build planning in `crates/ra-jit/src/cargo/whole.rs`, which
builds supported programs from source over the engine's standard library and
links them without running `cargo build` for the program.

`crates/hir-ty-codegen/patch.rs` tracks changed functions against a base object.
`crates/ra-jit/src/splice.rs` can append a patch to a linked executable, resolve
references against its symbols, and redirect old function entries to new code.
It starts from the linker's baseline rather than accumulating prior patches.
Statics that move require recompiling references. Object-format-specific paths
include `splice/macho.rs`, `splice/pe.rs`, and `splice/wasm.rs`. Unsupported splice
cases return errors and take a full link rather than silently emitting guessed
machine code. Full linking is still part of the system.

`crates/ra-jit/src/cargo/whole.rs` separates unit-test workers loaded with
`cfg(test)` from integration-test and ordinary-program workers without it.
`crates/ra-jit/src/cargo/run.rs` executes built programs/tests. Target selection,
working directory, test arguments, runtime environment and exit status are part
of the external contract. Documentation tests have a separate path in
`whole.rs`, not blanket evidence that all tests avoid Cargo/rustdoc tooling.

### Build scripts and proc macros are compatibility work, not exceptions

The loader uses rust-analyzer's project model and expansion infrastructure.
The historical wrapper path reruns build scripts and compares outputs in
`crates/ra-jit/src/cargo/script.rs`. The owned path in
`crates/ra-jit/src/cargo/own_script.rs` describes scripts and dependencies,
compiles them over the standard-library unit, links with `cc`, and writes
executables and dep-info where the loading machinery expects them. Its source
explicitly retains a rustc path for scripts that read compiler-only environment
variables. Correct `OUT_DIR`, emitted configuration/link directives, environment
reads, included files and rerun inputs remain obligations.

`crates/ra-jit/src/cargo/own_macro.rs` implements proc-macro compilation and
expansion. Its default is a Wasm module executed through
`crates/ra-jit-macro-wasm/`. Unsupported Wasm macros fail the load unless the
workspace explicitly selects native expansion via `native-proc-macros`.
Native expansion requires the compatible server in `vendor/proc-macro-srv/`.
The code also has explicit rustc-built proc-macro controls and isolated loading
artifacts. These modes are not interchangeable evidence of complete proc-macro
support. Native dependencies and platform ABI behavior must still be preserved.

## Explicit limits

Unsupported lowering must fail visibly with its reason, not guess semantics.
The source's `crates/ra-jit/src/cargo/whole.rs::refused` rejects, among other
cases, multiple targets, multiple `--test` selections, `--message-format`,
Cargo unstable flags, and `--target-dir` (while reading `CARGO_TARGET_DIR`).
The same file rejects unsupported staticlib/dylib builds, unit tests without
libtest's harness, and Wasm test execution. Inspect the exact source for current
selection rules rather than treating the new command family as full Cargo CLI
compatibility. Platform parsers accepting a target do not establish validated
execution on that platform. No full Rust-language, manifest, build-script,
proc-macro, ABI or platform compatibility is claimed here.

## Measured motivation, not a native integration speed claim

The November 1, 2026 corrected comparison in
`docs/three-stack-benchmarks.md` measured these median end-to-end seconds:

| Pinned filtered-test phase | Stock Cargo | Shipping justrust | Source-built upstream Blaze |
| --- | ---: | ---: | ---: |
| Reachable test-body edit | 1.079 | 1.086 | 0.258 |
| Cold build/daemon artifacts | 11.349 | 11.612 | 25.766 |

This was 126 correctness-gated calls across six stack-order permutations on a
pinned justrust snapshot and common toolchain. It was controlled-local, with
shipping-client remote execution and speculative prebuild disabled. Dependency
sources were populated before timing. Cold does not mean cold OS page cache or
a network download. The edited source was a filtered test body, not an arbitrary
production function. The reference disabled native/Wasm proc-macro-server modes
and used rustc-built proc macros, so it does not validate the modes above.

These figures compare the pinned upstream executable, not a fourth measured
stable native-engine stack. The experimental integration is not the shipping
client and cannot inherit the reference's numbers as its own results. Noise is
unknown and no statistical significance, general speedup, default-hosted win,
or default promotion follows. The reference was slower in this cold phase.
Methods and exact identities are in `docs/three-stack-benchmarks.md`, findings
in `FINDINGS.md` under the November 1 direct comparison, and all 42 shipping
recordings in `data/runs/three-stack-nospec-20261101/`, including
`20261101-123754213-3479372` and `20261101-123807946-3482115`.

Before promotion, measure the owned integration independently with the same
pinned workload and comprehensive session-weighted gate. Differential output,
diagnostics, invalidation, unsupported cases, recovery, cold state, test failures,
build scripts and proc macros need acceptance checks as well as timing.
