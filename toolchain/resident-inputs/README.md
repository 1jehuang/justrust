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
`-Ldependency=path` / `-L dependency=path`, plus `-Lnative=path` /
`-L native=path`. Externs must name regular `.rlib`
or `.rmeta` files. All such files in every dependency search directory are
included, not just named direct externs. Paths are absolute and lexically
normalized against cwd. Duplicate paths and directory ordering do not affect
the manifest. Directory paths themselves are retained, including empty ones.

Schema 2 adds a separate native search-directory list. Every regular file in
native directories is hashed, including object files and otherwise unrelated
files, so membership and content changes invalidate the snapshot. Native `.a`
files must start with the self-contained ar magic. Thin archives and linker
scripts disguised as archives are rejected because they can reference inputs
outside the closure. Empty native directories are retained too.

The safe subset rejects unresolved externs, conflicting extern names,
symlink path components or directory entries, dynamic-library entries (`.so`,
versioned `.so.*`, `.dylib`, `.dll`), `.lib` import libraries, nested directories
or other nonregular native entries, and search kinds other than dependency or
native. It does not execute proc macros. Platform/compiler inputs outside this
subset must use the caller's ordinary compilation fallback.

Limits are 2048 distinct files and scanned directory entries, 1 GiB of file bytes, and strictly less than
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
