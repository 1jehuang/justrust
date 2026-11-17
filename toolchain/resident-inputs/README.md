# jr-resident-inputs

Standalone shared crate for Cargo and the resident compiler. Call
`capture(args, cwd)` before retaining dependency state, retain the returned
manifest, and call `verify()` before reusing that state. Errors mean fallback
to ordinary compilation. `None` means there were no explicit extern inputs.

The serializable, cloneable, comparable `Manifest` has private fields and a
versioned schema. `key()` first verifies the snapshot, then hashes its bounded
canonical JSON encoding. Both methods return `Result<_, String>`. No public
unchecked construction or raw hash API is provided. Serialized manifests are
untrusted until verified. Compiler arguments, source, toolchain, environment,
and other compiler inputs must still be included in the caller's cache key.

Supported inputs are explicit `--extern name=path` / `--extern=name=path` and
`-Ldependency=path` / `-L dependency=path`. Externs must name regular `.rlib`
or `.rmeta` files. All such files in every dependency search directory are
included, not just named direct externs. Paths are absolute and lexically
normalized against cwd. Duplicate paths and directory ordering do not affect
the manifest. Directory paths themselves are retained, including empty ones.

The initial safe subset rejects unresolved externs, conflicting extern names,
symlink path components or directory entries, `.so` dependency-directory
entries, and non-dependency search kinds. It does not execute proc macros or
interpret artifact internals. Platform/compiler inputs outside this subset
must use the caller's ordinary compilation fallback.

Limits are 2048 distinct files, 1 GiB of file bytes, and strictly less than
1 MiB of serialized manifest data. Content is streamed into BLAKE3 in 64 KiB
chunks. Device/inode, length, modification time and change time are race
checks on Unix, not cache identities. Open-handle metadata and path metadata
are checked around reads, followed by directory membership recapture and
final path checks. As with any filesystem snapshot without an OS transaction,
callers must prevent writes after verification while consuming retained state.
Timestamp-only changes between captures preserve identity. Same-length byte
replacement, even with restored modification time, does not.

Validation (hosted only):

```sh
justrust check --manifest-path toolchain/resident-inputs/Cargo.toml
justrust test --manifest-path toolchain/resident-inputs/Cargo.toml
justrust clippy --manifest-path toolchain/resident-inputs/Cargo.toml -- -D warnings
```
