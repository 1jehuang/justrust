# Native Windows hosted builds

Windows execution is an explicit hosted platform, not a Linux cross-compilation
alias. The client requires an authenticated Windows worker response, a
short-lived SSH user certificate bound to its fresh public key, pinned SSH host
keys and a successful native agent probe before uploading source. It never
sends Windows work through the Linux shell, rsync or Unix daemon.

## Select Windows

```sh
justrust login
justrust remote use hosted --platform windows --target x86_64-pc-windows-msvc
justrust check
justrust test
justrust build --release --bin my-app
```

The target option on `remote use` is optional because x86_64 MSVC is currently
the only native Windows target. Other targets are rejected. To select Windows
for one command without changing the saved hosted platform:

```sh
justrust check --host-platform windows
justrust test --host-platform windows -- --nocapture
```

`JUSTRUST_HOST_PLATFORM=windows` is the equivalent process override.
`justrust setup set host-platform windows` persists a user-wide override using
the existing setup mechanism. `justrust setup unset host-platform` removes it.
`justrust remote up --platform windows` selects and starts a Windows worker
when the hosted backend is configured. To return to Linux:

```sh
justrust setup unset host-platform
justrust remote use hosted --platform linux
```

An environment/setup override takes precedence over `remote use`'s saved
selection. Cargo's `--target x86_64-pc-windows-msvc` alone does **not** select
Windows execution. Explicit Windows work never falls back to Linux, a local
prebuild, or a cached Linux test result, even when provisioning or compilation
fails. `--local` with a Windows platform is only valid on an actual Windows
client.

## Source, output and cache boundaries

* `check`, `test`, `build` and `clippy` are supported. Tests execute on the
  Windows worker, never on the Linux client. `run`, arbitrary Cargo commands
  and direct compiler invocations are not part of this transport.
* The client takes a fresh source snapshot from the normal workspace/path
  dependency roots, preserving relative sibling layout. Git-ignored source is
  excluded under the normal sync rules, except ignored Cargo lockfiles.
  Known credential paths such as `.env`, `.ssh`, `.aws` and Cargo credential
  files are excluded. Do not put secrets in source files or build configuration.
* Requests are bounded to 20,000 files, 8 MiB per file, 64 MiB decoded source and
  96 MiB JSON. Invalid Windows filenames, device names, traversal and
  case-colliding paths fail closed. This is not an uploader for large datasets
  or a substitute for reviewing which source a build may read.
* Caller-local `--manifest-path`, `--target-dir` and `--config` overrides are
  rejected. Invoke the command from the desired crate directory instead.
  Absolute machine-local paths in manifests/configuration are not rewritten.
* Only explicitly allowlisted public build controls are forwarded. Caller
  credentials, signing credentials and caller-local toolchain/SDK paths are
  not forwarded. The image supplies the reviewed native compiler and MSVC SDK.
* Windows enrollment, host keys and status are stored separately under
  `~/.justrust/remote/hosted/windows/x86_64-pc-windows-msvc/`. Linux state remains
  in its existing location. Each Windows connection refreshes enrollment rather
  than reusing an earlier authenticated SSH master.
* The native agent maintains Windows/target/project-scoped compiler output and
  creates a fresh source snapshot per run, so deleted files cannot persist in
  the next source tree. The control plane must dedicate the worker to the
  authenticated account. Client path separation is not a tenant sandbox.
* Successful builds return selected `.exe`, `.dll`, `.rlib` and `.lib` files,
  not a complete Cargo target cache or PDB/debug-symbol bundle. Every file is
  checked for SHA-256 and length. PE executables/DLLs additionally require the
  actual x64 machine type and appropriate executable versus DLL flags. Rust
  and static/import libraries require the ar archive signature. That signature
  verifies archive format only, not library architecture. The pinned native
  worker probe and target-scoped path bind their build provenance. Archives
  are stored as opaque bytes, never extracted or executed. Explicit binary
  builds and `test --no-run` still require a returned PE executable, so an
  archive cannot disguise a missing executable. Verified output is published
  atomically in a fresh per-run directory under
  `target/hosted-windows/x86_64-pc-windows-msvc/`. Linux output is never replaced.
* Transport output streams to the terminal and is saved in the local run log.
  A native recorder summary is imported when available. A missing terminal
  frame, invalid artifact or lost connection is an unknown/failed result, not
  permission to retry the build locally.

## Protocol and validation scope

The versioned transport is `windows-powershell-v1`. The fixed image-owned agent
is `C:\ProgramData\Justrust\hosted-agent.ps1`, invoked through PowerShell's
UTF-16LE `-EncodedCommand`. Source and arguments are JSON on stdin, never
interpolated into shell commands. The raw `-Probe` response binds schema 1,
Windows, the MSVC target and transport. Normal stdout is bounded NDJSON:
platform/run-bound `hello`, base64 `stdout`/`stderr`, checksum-bound chunked
`artifact-start`/`artifact-data`/`artifact-end`, then `exit`. The SSH exit code
must agree with the terminal frame. SSH exit 255 is always an unknown transport
result. No data may follow the terminal frame.

Client unit tests cover source layout/filtering, platform negotiation, strict
base64/framing, certificate key/validity binding, safe artifact paths and output
stream ordering. Hosted Linux compilation/tests validate the client code, not
native Windows provisioning, MSVC linking, worker containment or Windows
process cleanup. Those require the coordinated native-image/control-plane
acceptance workflow before claiming the service is operational for customers.
