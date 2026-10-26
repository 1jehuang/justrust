# Hosted portable GNU/Linux releases (glibc 2.17)

Verified October 26, 2026. The hosted Linux service can build full-feature Jcode
for the x86-64 glibc 2.17 baseline, without compiling inside a separate
manylinux machine. The baseline container below is a runtime acceptance gate,
not the build environment.

## Explicit supported command

```sh
JUSTRUST_REMOTE_REQUIRED=1 \
JUSTRUST_TOOLCHAIN=1.99.0 JUSTRUST_TOOLCHAIN_REQUIRED=1 \
CARGO_BUILD_RUSTC_WRAPPER='' \
CARGO_PROFILE_RELEASE_OPT_LEVEL=3 CARGO_INCREMENTAL=0 \
justrust zigbuild --remote --locked --release \
  --target x86_64-unknown-linux-gnu.2.17 \
  -p jcode --bin jcode --features linux-compat-vendored-openssl
```

Run from the Jcode checkout. Set the existing `JCODE_BUILD_GIT_HASH`,
`JCODE_BUILD_GIT_DATE`, `JCODE_BUILD_GIT_TAG`, `JCODE_BUILD_GIT_DIRTY`,
`JCODE_BUILD_CHANGELOG_RAW`, and `JCODE_BUILD_SEMVER` release metadata before
building. The verified trial used the actual source revision and dirty state,
not a made-up release tag.

The returned file is `target/x86_64-unknown-linux-gnu/release/jcode`: Zig's
`.2.17` ABI selector is not part of Cargo's artifact directory or Rust's std
library target. The exact selector remains in the build command. Other glibc
versions and ARM GNU targets are not admitted by this change.

Implementation `bc08a96d1` uses the existing pinned Zig 0.13.0 and
cargo-zigbuild 0.20.1 installation. The artifact reader checks x86-64 ELF and
its structured GLIBC version requirements before publishing the local file.
Missing, malformed, nonnumeric or newer GLIBC requirements are rejected.
Locally cached artifacts must pass the same version check before reuse.
The existing native GNU build is unchanged and does **not** promise 2.17.

## Real application acceptance

Source: Jcode `b411a720746abc18288e26a5a7ed159a0fd460fa`, clean checkout.
Default PDF and Bedrock features retained, with
`linux-compat-vendored-openssl` added. Recorded root library and binary
invocations both used optimization level 3 and incremental compilation off.

| Run | Observation |
| --- | --- |
| `20261026-150025317-548870` | Hosted build succeeded, 142.88 seconds build wall, 157.8 seconds client including sync and transfer. 702 units compiled. |
| `20261026-150308875-562781` | Warm repeat succeeded, 611 fresh units, no compilation. 3.4 seconds client including sync, already-current artifact reused. |

These are observations of this workload, not a controlled before/after speed
claim. The first run rebuilt dependencies for the changed compiler/linker
configuration. No broad benchmark or default-routing promotion is claimed.

Both runs produced the same SHA-256:

```text
b5b9329d67ec4a726f8127e72d85c636c2e5496c4d6e2b96bbcaa97f4ae9f97c
```

The highest imported GLIBC symbol version was **2.17**. The binary reported
`jcode v0.99.1 (b411a7207)` both on the local machine and in the baseline
container. In both environments these commands exited successfully:

```text
--no-update --no-selfdev --version
--no-update --no-selfdev cloud receive --help
--no-update --no-selfdev cloud credentials --help
--no-update --no-selfdev cloud router-credentials --help
--no-update --no-selfdev cloud cancel-return --help
```

Baseline runtime: the already-available image
`quay.io/pypa/manylinux2014_x86_64@sha256:2bd80fa8fca6b775a913b7e42fa410c5f2246fb6f7f8f3280b5d562e5ca5d63f`,
which reported `glibc 2.17`. It ran read-only, without networking, with
capabilities dropped, and as UID/GID 65534. Its `ldd` output resolved only
baseline libc, libm, libpthread, libdl and libutil, with no external OpenSSL
runtime library. The previous native-host trial binary failed in this same
image and required GLIBC 2.39. The new gate rejects that old binary before
execution.

Reproduce the runtime gate for a trusted build output:

```sh
bash ci/check-jcode-glibc217.sh /absolute/path/to/jcode
# If Docker access on this machine requires sudo:
sudo -n bash ci/check-jcode-glibc217.sh /absolute/path/to/jcode
```

The trial artifact is in `$JCODE_SCRATCH_DIR/jcode-glibc217-trial/jcode` on
the validation machine. It was not installed as the shared Jcode daemon.

## Regression gates and remaining scope

- Hosted `justrust test`: `20261026-145854935-539475`, **788 passed, 4 ignored**,
  no filtered-out tests. Includes strict selector/normalization, malformed ELF,
  too-new versions, and preservation of the old destination on rejection.
- Hosted check: `20261026-145906117-540518`, passed.
- Hosted clippy: `20261026-145909446-540809`, passed.
- The committed shipping client was installed locally for both application
  trials; no uncommitted source was installed.

This fixes the demonstrated Linux x86-64 glibc blocker. It does not publish a
Jcode release, replace the official release workflow, waive exact-tag metadata
or packaging checks, or claim native application test-suite coverage. macOS
SDK/signing, Windows MSVC/native execution/signing, and ARM64's existing
emulated-TLS contract are separate remaining platform work. The Windows GNU
path remains an additional ABI, not a replacement for MSVC.
