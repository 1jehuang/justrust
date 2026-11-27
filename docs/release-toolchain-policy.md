# Release compiler selection

Release compilation follows the hosted control plane, not a compiler hardcoded
in the client workflow. Preflight requests
`GET https://build.jcode.sh/v1/toolchain/current?release=1` once, fails closed on
HTTP/JSON/schema/protocol errors, and freezes the response plus the separate
bootstrap compiler into an immutable Actions artifact. The canonical JSON
SHA-256 is a job output. Every matrix job verifies that identity and the exact
candidate's `src/toolchain_policy.rs` supported protocol before building. There
is no per-job policy request, stale policy cache, moving `stable` channel, or
fallback to a system compiler.

The release response extends the ordinary `{toolchain, protocol}` response:

```json
{
  "toolchain": "jr-ac20de5e7921",
  "protocol": 1,
  "release": {
    "schema": 1,
    "targets": {
      "x86_64-unknown-linux-musl": {"toolchain": "jr-ac20de5e7921", "protocol": 1, "kind": "hosted-policy"},
      "aarch64-unknown-linux-musl": {"toolchain": "jr-ac20de5e7921", "protocol": 1, "kind": "hosted-policy"},
      "x86_64-apple-darwin": {"toolchain": "1.99.0", "kind": "stock-compatibility"},
      "aarch64-apple-darwin": {"toolchain": "1.99.0", "kind": "stock-compatibility"},
      "x86_64-pc-windows-msvc": {"toolchain": "1.99.0", "kind": "stock-compatibility"}
    }
  }
}
```

These IDs are illustrative, not workflow defaults. The ordinary endpoint without
`release=1` retains its original contract. The cloud's existing publication guard
still checks that the selected Linux host bundle is available before advertising
it. The release helper permits exactly the five supported release targets.

## Platform support, not a portability claim

The owned toolchain publication path currently verifies
`x86_64-unknown-linux-gnu` host bundles. Both static musl artifacts are cross-built
on the hosted Linux x86_64 machine using that selected compiler. Owned host-only
bundles provide their matching Rust library sources, so release builds use
`-Zbuild-std` to compile musl target libraries with that compiler rather than
mixing in upstream prebuilt Rust libraries. Those builds use the owned native
`rust-lld` linker. A separately pinned stock bootstrap sysroot supplies only the
11 allowlisted musl C ABI startup objects, libc and unwind archives. The helper
checks their architecture, format and hashes, stages them into the build inputs,
and binds the staged manifest to the frozen policy. No stock Rust `.rlib` files
are accepted. ARM retains the Cortex-A53 erratum workaround supported by the
native linker. The selected compiler builds all Rust libraries from its own
sources. Official exact compiler IDs retain the native musl build path with their
own target std. Missing sources
or an unusable compiler fail the build. This does not
establish owned compiler support on Darwin or Windows. Those three release
artifacts explicitly use the cloud's `RELEASE_COMPAT_TOOLCHAIN` exact stock
version, marked `stock-compatibility`. No moving aliases or jr IDs are accepted
for this compatibility path. The service must provide that mapping, otherwise
release preflight fails. Extending owned host support requires a separate bundle
and correctness audit, not changing a label here.

## Bootstrap is not the release compiler

The hosted transport client is the published release production selects when
preflight runs. It is frozen once per release run (tag plus published archive
SHA256, tag required on `stable` history), downloaded, and verified before use. Stock `1.99.0` remains frozen as `bootstrap`
independently of the service selection and supplies the C-only musl ABI inputs.
The helper checks the actual bootstrap compiler version before locating those
inputs. That compiler does not compile the Linux release binary. The candidate
release has `JUSTRUST_RELEASE_BUILD=1`. Release jobs explicitly override ambient/project
compiler selections with the frozen compiler and require toolchain installation
success plus hosted routing. They never modify service policy to make a build
pass.

## Provenance and existing gates

Linux jobs check their successful build's recorder metadata, requiring hosted
execution, the selected toolchain ID, and the actual recorded compiler path in
that toolchain prefix. `compiler-<target>.json` includes the recorded run ID,
compiler path, source revision, binary SHA-256 and complete frozen selection
with its SHA-256. Owned compiler receipts also include the C-only input manifest
and require the recorded build command to reference that exact staged path.
This records the
compiler actually used, rather than merely echoing the requested environment.
It is recorder provenance, not a hardware-backed or signed supply-chain
attestation. A missing/mismatching recording refuses release packaging.
Compatibility jobs record `cargo --version`, `rustc -vV` from the explicitly
selected rustup toolchain, compatibility mode and the frozen policy digest.

The existing exact-tag-commit build identity, stable ancestry and package
version checks, static ELF checks, native/QEMU version/help smoke tests, archive
checksums, and final moved-tag refusal remain in place. Full CI/test/performance
acceptance follows publication under the shared production-first policy in
`docs/toolchain-release-flow.md`, not a successful-CI prerequisite. Service
authentication, billing and tenant isolation are unchanged. No workflow validation command
publishes a client or compiler, tags a release, or activates an installation.

Run the local policy checks with:

```bash
python3 ci/test-release-policy.py
actionlint .github/workflows/release.yml
bash -n ci/install-hosted-client.sh ci/check-linux-release.sh
```

A real hosted release-target build with the selected live compiler is also
required before considering a release-policy change validated. Preserve the
build's recorded run ID and check it through `justrust show <id>`.
