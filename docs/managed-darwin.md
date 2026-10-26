# Managed Darwin cross-build SDKs

The Darwin helper supports `x86_64-apple-darwin` and `aarch64-apple-darwin`.
It reuses the pinned Zig 0.13.0 and cargo-zigbuild 0.20.1 installation in
`release_tools`, not an arbitrary Zig found on the host PATH.

## Explicit host-local provisioning

Apple SDK material is **never downloaded by this helper**. It does not accept
licenses, acquire signing identities, or imply that a particular SDK license
permits any particular use. An administrator must independently obtain an SDK
and establish permission to use it in the intended hosted environment.

Run provisioning locally in the same hosted account that will build. The helper
API is deliberately separate from build request handling:

```rust
// Host-local administrative operation, not an SDK path supplied by a remote job.
let sdk_id = darwin_tools::provision(std::path::Path::new("/admin-provided/MacOSX.sdk"))?;
// Persist the returned digest as the account's explicitly selected SDK ID.
```

The integrated CLI exposes the same administrative operation. Run it on the
hosted host, once the updated client is installed:

```sh
justrust toolchain import-darwin-sdk /admin-provided/MacOSX.sdk --licensed
```

`--licensed` is the administrator's confirmation of existing rights, not automatic
license acceptance. Select the printed digest with `JUSTRUST_DARWIN_SDK` for
`justrust zigbuild --release --target aarch64-apple-darwin` (or x86_64).
Do not forward a laptop SDKROOT or an arbitrary filesystem path to a hosted
worker, and do not upload SDK material as project source, cache artifacts, or
build-request attachments.

The account-local store is `<toolchain root>/apple-sdks/v1/<sha256>/`, containing
`manifest.json` and `sdk/`. The manifest has schema version 1 and a sorted mapping
of relative regular-file names to SHA-256 hashes. Its exact bytes determine the
SDK ID. Provisioning stages a complete copy before publishing by rename. Existing
IDs are validated and never overwritten. Files and manifest are marked read-only.
SDK upgrades produce new IDs rather than modifying a published SDK.

Internal SDK symlinks are materialized into ordinary files/directories. Escaping
symlinks, directory cycles, devices, sockets and other special files are rejected.
Source SDK changes therefore cannot silently change a published SDK. Keep the
store under trusted account administration, not writable by untrusted build jobs.
Read-only bits and content hashes are not a sandbox against an administrator or
a process with the same filesystem privileges. Integrity validation and build
execution must run without a concurrent untrusted writer to this store.

Each application rehashes the manifest and the entire SDK file inventory, rejecting
modified, added or missing files and runtime symlinks. This deliberately favors
correctness over a metadata-only cache. It makes no validation-overhead or build
speed claim. Minimal completeness checks require SDKSettings JSON or plist,
stdio.h, stdlib.h, TargetConditionals.h, libSystem.tbd, and Foundation,
CoreFoundation and AppKit framework stubs. Each checked stub must name macOS and
both x86_64 and arm64. These are preflight checks, not full SDK semantic validation.
The real compiler/linker remains authoritative for all additional dependencies.

## Owned build command setup

```rust
assert!(darwin_tools::supported(target));
// The caller selects cargo-zigbuild and passes --target <target>.
darwin_tools::apply(&mut command, target, &sdk_id)?;
```

`apply` accepts only a lowercase 64-character SHA-256 SDK ID, never a path.
It validates the managed SDK before acquiring release tools. It configures
`SDKROOT` only on the owned command, overriding any inherited SDKROOT, and applies
the release tool pin, Zig binary override and native-linker-interception disable.
It does not mutate the agent process environment. Release tools retain their
existing download policy, including `JUSTRUST_TOOLCHAIN_AUTO_INSTALL=0`; that
policy never authorizes an Apple SDK download. Callers must install the matching
Rust standard library target and include the SDK ID in any cross-build cache
identity. Preserve failures rather than silently choosing a different SDK.

No certificates, private signing keys, passwords, keychains or notarization
credentials belong in this toolchain store. Signing and notarization are separate
explicitly authorized release operations, outside this helper.

## Verification scope

Tests cover strict targets/IDs, repeatable content IDs, required architecture
checks, inventory mutations and source symlink escape rejection. A full Jcode
cross-build requires a lawfully provisioned SDK on the actual hosted account.
No SDK was found in the specific local conventional `/opt`, `/usr/local/share`,
or CommandLineTools SDK locations checked during implementation. Do not report a
full SDK-backed cross-build unless one was actually run. Even successful Linux
cross-compilation verifies neither execution on macOS nor signing/notarization.
Run the resulting binaries and relevant tests on real macOS machines separately.
