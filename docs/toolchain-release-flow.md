# Owned-source toolchain release flow

This public entrypoint implements **BUILD only**. It does not publish, select a
production default, deploy services, or install a client. Packaging and shared
production activation belong to the private cloud repository.

## Production-first release policy

Release order is **bump -> commit -> build/package sanity -> tag/publish ->
shared production -> test/measure -> fix forward**. Tests and benchmarks
observe the globally published, activated release, not an unpublished candidate.
Every customer uses the same shared production default. There is no canary,
per-customer validation selection, successful-CI prerequisite, or 40-point
benchmark gate before publication. A release is not declared correct or faster
merely because publication succeeded.

Before publication retain source/version identity, package completeness,
checksums, executable identity, and runtime loader sanity checks. These checks
establish that the intended artifact can run, not test-suite or performance
acceptance. After activation, run correctness tests and representative workload
measurements using the published artifacts. Record failures and measured
regressions, then publish a new immutable fix-forward release. Failed toolchain
validation never automatically switches production back. An explicit emergency
recovery remains an operator action, not the normal iteration loop.

## Plan first

On `stable`, **bump `toolchain/version.toml` first**, then commit the complete
intended source and run:

```sh
python3 scripts/toolchain-candidate.py --source-commit "$(git rev-parse HEAD)"
```

The default is JSON on stdout with no build or filesystem writes. The required
revision is the full lowercase 40-character **repository** commit, exactly HEAD.
The immutable candidate name is `jr-<full repository commit>`, never an upstream
commit read from the vendored `toolchain/rustc/git-commit-info`. The plan also
reads the committed `toolchain/version.toml` and records its numeric version and
planned `toolchain-vVERSION` release tag. BUILD does not create or push a tag.

Toolchain tags use `toolchain-vVERSION`; client releases independently bump root
`Cargo.toml` and use `vVERSION` tags through the client release pipeline. Tag the
exact committed source identified by BUILD after build/package sanity, without
waiting for correctness tests or benchmark acceptance. Never move an existing
release tag. This documentation does not bump or release either component.

The entrypoint refuses staged or unstaged tracked changes under `toolchain/`,
the source/candidate scripts, root Cargo manifests/lockfile, `build.rs`, and
`.cargo/`. Unrelated documentation changes do not block it. Untracked files and
ignored build output are never included in the committed source snapshot.
Concurrent compiler edits must be committed by their owners before building.
Do not reset or clean another agent's files to pass this gate.

## Explicit build

```sh
python3 scripts/toolchain-candidate.py \
  --source-commit "$(git rev-parse HEAD)" \
  --output-dir /path/to/scratch/toolchain-candidates --execute
```

Python 3.11+ with tarfile's `data` extraction filter, Git, Bash, the compiler bootstrap
prerequisites, and the installed pinned `nightly-2026-10-08` justrust toolchain
are required. This is an expensive opt-in compiler build. No interactive input
is supplied, and helper output goes to stderr so stdout remains machine JSON.
The first compiler bootstrap and the pinned Cargo helper start concurrently from the same immutable snapshot, with separate output directories. Both must finish successfully before PGO training, BOLT, or artifact acceptance. Previously released Cargo binaries are not substituted merely because the source trees match. The helpers are:

1. `bash toolchain/rustc/build-jr.sh release`, with `JR_STAGE=1 JR_TOOLS=1`.
2. `bash toolchain/cargo/build-jr.sh nightly-2026-10-08`, with
   `JR_CARGO_FEATURES=all-static` and a pinned absolute `CARGO_TARGET_DIR`
   inside the candidate's `cargo-source/` snapshot.

The Cargo helper runs wherever justrust routes builds on this machine: hosted
by default, and always when local compiles are off (`justrust local off`). A
hosted build fetches the release binary back, and the stage fails right away
if no executable arrives. It never forces a local compile.
Both helpers run with `CI=1`, `GIT_TERMINAL_PROMPT=0`, and closed stdin.

The candidate directory must not already exist, even after a failed attempt.
The script exports `git archive <full commit>` into a non-Git `source/` snapshot,
not a worktree. It builds there, never in the concurrently edited checkout.
jr-cargo builds from `cargo-source/`: the same commit's `toolchain/` without
`toolchain/rustc`, under an empty `git init`. The hosted mirror root is the
nearest `.git` ancestor, and jr-cargo reads `toolchain/shared` and
`toolchain/resident-inputs` outside its own workspace, so that root keeps them
in the mirror without uploading the compiler tree.
Before building, only the snapshot's rustc `git-commit-info` is set to the
integrated repository commit, short hash and commit date. This is explicit
archive version metadata, not a claim that the old vendored upstream identity
already matched our repository. Cargo retains its existing `JUSTRUST_BASE`
identity convention and source fingerprint behavior. No live tracked source or
metadata is rewritten.

`provenance.json` pins `source_revision`, candidate ID, source metadata,
commands, working directories, environment overrides, and artifact paths. It
records `built` only after both helpers succeed, executables exist, and the
actual compiler's `rustc -vV` reports exactly the integrated source commit.
Mismatched compiler identity fails rather than being relabeled during packaging.
Failed attempts retain their provenance. The checkout is revalidated before
each helper and after the build, so concurrent relevant edits fail closed.

## Package, publish, and activate privately

Transfer the built compiler sysroot, jr-cargo binary, and provenance together to
the private packaging workflow. Run private `toolchain_publish.py` with
`--require-phase-ledger --require-jr-cargo`, using the exact candidate ID and
artifact paths recorded by BUILD. Consult that tool's `--help` for the current
required path and upload arguments. The phase ledger and Cargo are mandatory,
not optional ways to bypass a packaging failure. Verify the packager's observed
compiler commit against BUILD's recorded `rustc_verbose_version` and source SHA.
BUILD provenance is not a substitute for the publisher's artifact identity,
checksum, package completeness, and runtime loader sanity checks.

The private production-first controller publishes and activates the shared
production default before correctness tests or performance measurements. The
`toolchain-vVERSION` tag must resolve to BUILD's exact full source revision and
its committed version file must match the requested release version. There is
no canary stage or prepublication correctness/benchmark promotion gate. Do not
edit the production index manually or interpret BUILD success as production
activation. This public BUILD script makes no production changes.

## Client acceptance after publication

The client workflow builds and packages the tagged source and creates its GitHub
release without waiting for a successful CI run. The service must select that
published release as the shared production default before acceptance starts.
Download the published client, verify its checksum, release marker, version and
production selection, and use that exact published CLI to run the full client
suite against the tagged source (`justrust test --remote --locked`). Never
install an unpublished source build as a substitute. Keep full failing output
and run receipts, and measure real workloads after activation.

`scripts/production-dogfood.py` already checks the selected production tag,
download checksum, release marker and exact executable bytes. Its current
`--activate` path runs a smoke workload before replacing the local PATH client,
then another smoke workload afterwards. This is post-publication artifact
verification, not a global publication gate and not full client-suite coverage.
The release workflow calls `.github/workflows/client-production-tests.yml` as
soon as the Linux builds of that run have published (`linux-published`). It
does not wait for macOS or Windows runners, which can stay queued for hours.
The legacy latest pointer still moves only after every platform has published.
A newer release cancels older Release runs that have nothing left but queued
macOS or Windows builds (`ci/release-run-gates.py`). The production test
workflow waits for shared production selection, downloads the
verified published executable, and runs the full tagged-source suite through it.
The smoke helper alone is not full-suite acceptance.

If post-publication tests or measurements fail, report the release and receipts
and publish a fix-forward release. Never silently replace it with an unpublished
binary or alter account-specific routing to hide a regression.

## Tests and limits

```sh
python3 scripts/test_toolchain_candidate.py
```

Tests use temporary Git repositories and mocked build helpers. They cover plan
JSON, full-SHA/branch/dirty-source refusal, snapshot provenance, helper invocation,
failed build records, immutable attempt directories, and actual compiler identity
mismatch. They do not compile a compiler, validate the complete release package,
or establish a performance improvement.

The SHA is an immutable source identity, not a reproducible binary digest.
Bootstrap downloads, host dependencies, and inherited build environment can
still influence artifacts. Only explicit environment overrides are recorded.
Build directories are retained for packaging and debugging and are not
automatically cleaned. A failed attempt must be investigated and retained or
moved aside explicitly before retrying that same source identity.
