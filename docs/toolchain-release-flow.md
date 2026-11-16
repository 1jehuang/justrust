# Owned-source toolchain release flow

This first iteration implements **BUILD only**. It does not publish, select a
production default, deploy services, install a client, or run a canary. Packaging
and direct gated promotion belong to the private cloud repository.

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

Release order is version bump, source commit, build and validate, tag the exact
validated source commit, publish, then activate. Toolchain tags use
`toolchain-vVERSION`; client releases independently bump root `Cargo.toml` and
use `vVERSION` tags through the existing client release pipeline. Do not tag an
unvalidated candidate or move an existing release tag. This change initializes
the toolchain's next candidate version at `0.1.1`; it does not change the client
version or release either component.

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
The existing helpers run, in order:

1. `bash toolchain/rustc/build-jr.sh release`, with `JR_STAGE=1 JR_TOOLS=1`.
2. `bash toolchain/cargo/build-jr.sh nightly-2026-10-08`, with
   `JUSTRUST_REMOTE=off JR_CARGO_FEATURES=all-static` and a pinned absolute
   `CARGO_TARGET_DIR` inside the candidate snapshot.

Local Cargo execution is intentional here: the helper warns that hosted builds
may leave no installable local binary, whereas packaging requires that binary.
Both helpers run with `CI=1`, `GIT_TERMINAL_PROMPT=0`, and closed stdin.

The candidate directory must not already exist, even after a failed attempt.
The script exports `git archive <full commit>` into a non-Git `source/` snapshot,
not a worktree. It builds there, never in the concurrently edited checkout.
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

## Package, validate, and promote privately

Transfer the built compiler sysroot, jr-cargo binary, and provenance together to
the private packaging workflow. Run private `toolchain_publish.py` with
`--require-phase-ledger --require-jr-cargo`, using the exact candidate ID and
artifact paths recorded by BUILD. Consult that tool's `--help` for the current
required path and upload arguments. The phase ledger and Cargo are mandatory,
not optional ways to bypass a packaging failure. Verify the packager's observed
compiler commit against BUILD's recorded `rustc_verbose_version` and source SHA.
BUILD provenance is not a substitute for the publisher's artifact checks,
phase-ledger requirements, correctness gates, or benchmark evidence.

Promotion is a separate, explicitly authorized operation through private
`tools/promote-toolchain.py`, directly gated by its validation evidence. Before
promotion, the `toolchain-vVERSION` tag must resolve to BUILD's exact full source
revision and its committed version file must match the requested release version. There
is no canary stage in this flow. Do not edit the production index manually or
interpret BUILD success as permission to promote. This public script never
invokes either private tool and makes no production changes.

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
