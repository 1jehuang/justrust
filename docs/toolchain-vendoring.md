# Owned toolchain sources

justrust owns its compiler and Cargo as ordinary source in this repository.
`toolchain/rustc` contains rustc, std, bootstrap, and the imported tool sources.
`toolchain/cargo` contains Cargo. Bootstrap's `toolchain/rustc/src/tools/cargo`
is a shared-source symlink to `../../../cargo`, not a second copy or gitlink.
`toolchain/rustc/library/backtrace` is imported source too. There are no nested
Git repositories to initialize for these components.

`jr-rustc` and `jr-cargo` remain component and binary names. They do not denote
separately pinned development repositories. Make coordinated changes here,
including the client, native toolchain components, tests, and cache protocols.
There is no required upstream rebase or separate-fork release workflow. Adapt
upstream fixes when useful, subject to the external correctness contract in the
root `AGENTS.md`.

## Import provenance and policy

`toolchain/sources.json` records import provenance, the shared Cargo mapping,
optional source revisions, and frozen CI LLVM provenance. Its import hashes
record where the initial source came from, not the identity of every later
build and not pins to separate active forks.

| Imported source | Import revision | Upstream base, where recorded |
| --- | --- | --- |
| rustc | `d907fba494adcabfdd642a18a7918b0c9847d35b` | `1d81eb4ad9cd207e3e638bd32b17ec4fce8412a6` |
| Cargo | `a45c3546671925c24649e451a673aa71b1385d0d` | `f3865b2a4d1acc5276f6b3c67d0e057f4dab3928` |
| backtrace | `d902726a1dcdc1e1c66f73d1162181b5423c645b` | Not separately recorded |

The manifest also retains `previous_pin` fields for migration provenance.
These are not instructions to restore the old submodules. Historical dated
measurement documents and results retain the paths and revisions actually
used at measurement time.

The original `~/jr-rustc` and `~/jr-cargo` repositories were left intact.
The import exported their tracked source, not their Git metadata or build
directories. Licenses and technical fixtures were audited against the original
tracked blobs, including ignored fixture directories and CRLF inputs.

The root `AGENTS.md` is authoritative throughout `toolchain/`. Inherited agent,
contribution, AI-use, and separate-repository workflow policies were removed.
Policy landing pages required by documentation navigation point to the root
policy rather than imposing upstream governance. Technical implementation
and build documentation, licenses, attribution, documentation inputs, and test
fixtures remain. Keeping technical documentation is not an obligation to keep
upstream architecture or internal protocols.

## Build the owned components

Run commands from the justrust repository root unless a subshell changes the
working directory. Inspect the helpers before using advanced modes.

### Cargo

```sh
(cd toolchain/cargo && ./build-jr.sh)
# Equivalent explicit compatibility toolchain:
(cd toolchain/cargo && ./build-jr.sh nightly-2026-10-08)
```

`toolchain/cargo/build-jr.sh` defaults to `nightly-2026-10-08`. It reads the
installed toolchain's Cargo release and channel, then uses `justrust build`
with matching `CFG_RELEASE` and `CFG_RELEASE_CHANNEL`. The default
`JR_CARGO_FEATURES=all-static` builds the native dependencies statically.
Setting `JR_CARGO_FEATURES=` instead requests system-library linkage.

Hosted justrust builds are the normal path. A hosted build can succeed without
leaving an installable local binary. When a local artifact is specifically
required, the helper documents the local-artifact exception:

```sh
(cd toolchain/cargo && JUSTRUST_REMOTE=off ./build-jr.sh nightly-2026-10-08)
```

A local release binary normally lives at
`toolchain/cargo/target/release/cargo`, or under `CARGO_TARGET_DIR` when set.
Installation is an explicit operation, separate from building. Set
`TOOLCHAIN_ID` to the intended installed toolchain before running:

```sh
justrust toolchain cargo-fork toolchain/cargo/target/release/cargo --toolchain "$TOOLCHAIN_ID"
```

Do not install or activate an incompletely validated migration candidate.

### Compiler, std, and matching tools

```sh
(cd toolchain/rustc && ./build-jr.sh dev)
(cd toolchain/rustc && ./build-jr.sh release)
# Build stage 2 plus matching rustdoc and clippy for an installable sysroot:
(cd toolchain/rustc && JR_STAGE=2 JR_TOOLS=1 ./build-jr.sh release)
```

The compiler helper defaults to `release` when no mode is supplied and to
stage 1 unless `JR_STAGE` overrides it. `dev` enables incremental compilation
and thin-local LTO. `release` uses nonincremental thin LTO and one codegen unit.
`JR_TOOLS=1` adds rustdoc and clippy to the library and compiler build so the
tools match the driver. The helper also supports PGO modes and BOLT preparation,
which require their respective profiles and training steps.

Bootstrap writes `build/host/stage1` by default, or `build/host/stage2` for the
stage-2 invocation. The helper saves a copy under `build/jr/<mode>`, with a
`-stage2` suffix for stage 2. These copies let benchmark modes coexist.
`justrust toolchain jr-install` defaults to
`~/justrust/toolchain/rustc/build/host/stage2`. A stage-1 build alone does not
produce that default install sysroot. Preserve the distinction between the
benchmark stage-1 examples and the stage-2 installation default.

For a validated candidate, stage it without changing the default compiler:

```sh
justrust toolchain jr-install toolchain/rustc/build/host/stage2 --no-default
```

Compiler bootstrap uses its native `./x` build workflow, not an interchangeable
root `justrust build` invocation. Ordinary root Rust checks, tests, and lints
continue to use hosted justrust.

## Prebuilt LLVM and source prerequisites

`toolchain/rustc/justrust.bootstrap.toml` disables submodule management and
uses prebuilt CI LLVM. The manifest freezes the CI artifact provenance to Rust
commit `1d9e68013afd163e05032b507400ff74d007ed72`, LLVM source commit
`aab11ee0ea5ea9a1d462ad0bc1fae9d9bb643019`, with assertions disabled.
Inspection of the actual CI LLVM payload found compiler-rt sources for both
profiler support and optimized builtins, including `compiler-rt/lib/builtins`.
The initial full-build failure was a native source-path resolver bug, not an
incomplete CI payload. The shared native CI/local compiler-rt resolver now serves
both consumers without changing their optimized-builtins or profiler flags.
It requires `lib/builtins`, `lib/profile`, and `include` before selecting CI
sources, otherwise falls back to explicit LLVM checkout guidance. The default
full compiler/std build and smoke checks passed without any LLVM source checkout.

For an optional LLVM source build or other experiments needing the checkout,
inspect available entries and materialize the explicit LLVM revision using
either command below. The `--from-repo` variant reads an existing repository
without modifying it. This roughly 2 GB checkout is not required for the default
validated CI-LLVM compiler/std build:

```sh
python3 scripts/toolchain-source.py list
python3 scripts/toolchain-source.py llvm
# Reuse an existing repository containing the exact manifest revision:
python3 scripts/toolchain-source.py llvm --from-repo /path/to/llvm-repository
```

The materializer exports the explicit full commit in `toolchain/sources.json`,
not a moving branch. It imports archive content without nested Git metadata or
inherited policies, refuses nonempty destinations, and rejects nested gitlinks
that need explicit handling. It never replaces the owned rustc or Cargo trees.
Other manifest entries cover GCC, Enzyme, performance tools, and documentation
sources. Download caches and temporary Git data live outside the destination
source tree.

Owned compiler and Cargo source does not mean every optional dependency or
build prerequisite is vendored, nor that a clean build is offline. Stage-0
artifacts, CI LLVM, Cargo registry dependencies, native build tools, and optional
source-build dependencies may still need downloads or system provisioning.
LLVM source materialization alone does not select an LLVM source build. Configure
bootstrap separately if that is the intended experiment.

## Native identity and cache separation

Import provenance is distinct from the identity of current owned source.

- Compiler bootstrap recognizes the owned source using its monorepo marker and
  `toolchain/sources.json`. Its native `OwnedSource` identity is the first
  40 hexadecimal characters of a SHA-256 content digest over the selected
  compiler, library, bootstrap, build-helper, shim, workspace, manifest, version,
  channel, and bootstrap-configuration inputs. The short identity is nine
  characters. Dirty owned-source edits change the identity before committing.
  Identical source in a no-Git monorepo archive retains the same identity.
  Generated output directories do not supply the identity. This is neither
  managed nested Git nor the upstream release-tarball default path.
- Cargo retains the upstream compatibility base from `JUSTRUST_BASE` in its
  public commit fields. Its separate `jr-cargo` fork suffix is the first 40
  hexadecimal characters of a BLAKE3 content digest over selected owned Cargo
  inputs, including source, crates, credentials, manifests, build script,
  configuration, and packaged manual inputs. Dirty edits and no-Git source
  mirrors are distinguished without substituting the enclosing repository's
  commit for Cargo's compatibility base.
- The frozen CI LLVM artifact provenance is independent of these content
  identities. Editing owned compiler source must not accidentally change the
  LLVM artifact selected for download.

These native identities distinguish owned-source changes for existing identity
consumers. They are not interchangeable with the recorded import Git hashes.
Toolchain versioning and effective flags still belong in artifact cache keys.

## Observed migration validation, October 14, 2026

Root run summaries are exported under
`data/runs/xps14-toolchain-vendoring/2026-10.jsonl`:

| Check | Recorded run ID | Observed result |
| --- | --- | --- |
| Hosted `justrust check` | `20261014-062943813-3583068` | Passed, no diagnostics |
| Hosted `justrust test` | `20261014-062945291-3583240` | 439 passed, 3 ignored, 0 failed |
| Hosted `justrust clippy --all-targets -- -D warnings` | `20261014-062958361-3583001` | Passed, no warnings |

The coordinator also reported root formatting checks and verification of the
installed client's migrated default sysroot path. The migration workers
reported 23 bootstrap Python tests and 10 source-materializer Python tests
passing. Final native bootstrap validation passed all 289 tests, including
nine owned-identity tests and six compiler-rt selection tests. The six selector
tests also passed in isolation. No expected snapshots were changed. Durable
redacted native stdout/stderr is retained in
`data/toolchain-vendoring/2026-10-14-native.txt`.

Cargo validation receipts are included in the same isolated run file:

| Check | Recorded run ID | Observed result |
| --- | --- | --- |
| Hosted `check --bin cargo --locked` | `20261014-063101764-3590792` | Passed |
| Final actual `build --bin cargo --locked` | `20261014-070837591-3919225` | Passed locally through justrust's debug-artifact route |
| Hosted `test --test jr_build_identity --locked` | `20261014-065210842-3760408` | Four passed, including credential and native-manifest edits |
| Final hosted `test --test testsuite version:: --locked` | `20261014-070929167-3919209` | 31 passed |
| Broader `test --test testsuite version --locked` | `20261014-064035177-3661945` | 167 passed, one cross-target environment failure |

Each Cargo command supplied scalar CLI configuration
`--config 'env.CFG_RELEASE="1.101.0-nightly"'`
and `--config 'env.CFG_RELEASE_CHANNEL="nightly"'` from the Cargo directory.
The broader substring filter also selected `artifact_dep::index_version_filtering`,
which failed because `i686-unknown-linux-gnu` std was unavailable. This is not
claimed as a full Cargo suite pass. The narrower `version::` run passed all
selected tests. The built binary retained the upstream compatibility base and
reported its separate `jr-cargo` content-derived suffix.

The first full dev build compiled the stage-1 compiler and Cranelift but exposed
the optimized-builtins resolver gap. After the native fix and final bootstrap
tests, `./build-jr.sh dev` completed successfully in 149.04s, building the
compiler, both codegen backends, and optimized std. This was an incremental
validation rerun, not a cold-build performance result. The resulting compiler
reported source identity `47ab1d17fd84ef646af7c065f1c77c84e04ded4e`.

A metadata-only check and actual test execution passed on the rebuilt compiler.
The committed fixture exercises u128 division/remainders, std allocation/string
operations, and threads/synchronization. All three tests passed with LLVM and
again with Cranelift. The saved `build/jr/dev` compiler reported the same
identity. Reproduce the representative smoke from the repository root:

```sh
compiler=toolchain/rustc/build/host/stage1/bin/rustc
out="${JCODE_SCRATCH_DIR:?}/toolchain-smoke"
mkdir -p "$out"
"$compiler" -vV
"$compiler" --edition=2024 --crate-type=lib --emit=metadata \
  tests/fixtures/toolchain_smoke.rs -o "$out/smoke.rmeta"
"$compiler" --edition=2024 --test tests/fixtures/toolchain_smoke.rs -o "$out/llvm-tests"
"$out/llvm-tests" --nocapture
"$compiler" -Zcodegen-backend=cranelift --edition=2024 --test \
  tests/fixtures/toolchain_smoke.rs -o "$out/cranelift-tests"
"$out/cranelift-tests" --nocapture
```

This is representative Linux stage-1 build/smoke acceptance, not the full Rust
language suite, a stage-2 distribution release, or cross-platform acceptance.
No compiler candidate or new Cargo binary was installed or activated.
No compile-time speedup is claimed by this migration. Performance claims require
the before/after workload measurements specified in the root `AGENTS.md`.

## Stage source deliberately

Commit the source import, identity changes, client paths, policies, provenance,
and tests as one coordinated migration, preserving unrelated work. Inspect the
exact staged paths. Some Cargo tests intentionally contain source fixtures
under directories named `target`. Such fixtures may need narrowly scoped force
adds because generic build-output ignores would otherwise omit them.

Do not force-add all of `toolchain/`: compiler `build/`, Cargo `target/`,
download caches, generated bootstrap configuration, and other build outputs
must not become part of the source import. Preserve technical fixtures while
excluding actual generated artifacts. Optional materialized sources need the
same deliberate review rather than blanket staging.
