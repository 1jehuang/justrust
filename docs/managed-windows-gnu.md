# Managed Windows GNU builds on Linux

Windows GNU is an **explicit opt-in additional target**, not a change to the
existing Windows MSVC release ABI. The managed build host must be x86_64 Linux.
Normal `justrust test` behavior is unchanged. Cross-target tests must use
`--no-run`; Windows executables are never run on the Linux worker.

## Compile remotely, run on the target

```sh
justrust test --remote --no-run --target x86_64-pc-windows-gnu \
  --message-format=json > windows-tests.jsonl
# Continue only if compilation and artifact download both succeeded.
justrust test-bundle pack --target x86_64-pc-windows-gnu \
  --messages windows-tests.jsonl --artifact-root "$PWD/target" \
  --out windows-tests
```

The hosted path returns only Cargo-selected test executables, including fresh
ones. Returned Cargo artifact paths refer to the downloaded local files, not
remote filesystem paths. The build record and bundle say **not tested**.
Transfer the bundle to a matching x86_64 Windows machine, then run:

```sh
justrust test-bundle run --bundle windows-tests --results results.json
```

Only run code you trust. This command is not a sandbox. See
[cross-test-bundles.md](cross-test-bundles.md) for checks, exit codes, runtime
requirements, immutable build status and separate test results. Target runtime
DLLs, test resources and source fixtures are not bundled automatically.

Ordinary `justrust build --remote --release --target x86_64-pc-windows-gnu`
is also an explicit cross-build request. Existing MSVC or macOS release jobs
are not replaced by GNU artifacts.

## Provisioning and cache separation

The selected Rust compiler and target standard library must match. The normal
pinned-toolchain path installs that compiler's Windows GNU target library.
With automatic installation enabled, the Linux worker installs seven
checksum-pinned Ubuntu MinGW packages into a versioned account-local tool
store. `dpkg-deb` extracts them without root privileges or maintainer scripts.
HTTPS download tools, checksum verification, `dpkg-deb`, and compatible host
runtime libraries are required. Package licenses and notices remain in the
extracted tree. No Microsoft or Apple SDK is downloaded or accepted.

Only target-specific `CC`, `CXX`, `AR` and Cargo linker variables are set.
Host build scripts and proc macros retain their native compiler. The managed
path disables justrust's native linker override for this foreign target.
`JUSTRUST_TOOLCHAIN_AUTO_INSTALL=0` refuses missing tools rather than installing
them. `JUSTRUST_TOOLCHAIN=system` opts out of the managed compiler/toolchain
path and leaves user-supplied cross tools responsible for correctness.

Compilation remains inside normal hosted admission, isolation and billing.
Downloaded artifacts are checked for the requested Windows PE architecture;
that is not proof of runtime correctness. Cross-compilation does not remove
native dependency, linker, ABI, runtime, signing or platform validation needs.

## Scope

This path does not provide Windows MSVC, Apple SDKs, arbitrary Rust targets,
platform signing, or runtime testing on Windows. GNU and MSVC artifacts are
not interchangeable. macOS and MSVC require their own supported toolchain,
SDK/licensing and validation arrangements before admission can be enabled.
