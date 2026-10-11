# Hot patching of test binaries (prototype, FINDINGS 28)

Not wired into justrust, not default-on. A measurement prototype for phase 5
of `docs/toolchain.md`: after a body-only edit, rerun tests without
relinking the test binary.

```sh
cargo build --release --manifest-path bench/hotpatch/Cargo.toml
# use it on any crate: link and run tests through jr-hot
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=$PWD/bench/hotpatch/target/release/jr-hot
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER="$PWD/bench/hotpatch/target/release/jr-hot run"
export JR_HOT_REAL_LINKER=cc   # what a full link runs (a project's fast-linker script, ...)
JR_HOT=1 justrust test ...     # first run links a base, later edits patch
# A/B timing on private copies (see bench.py for the workspaces it expects)
flock /tmp/justrust-bench.lock bench/hotpatch/bench.py desktop -n 3
```

How it works:

1. **Base link.** With `JR_HOT=1`, the first link is a normal link plus
   `--no-gc-sections` (a later patch may call an instance nothing used
   yet). jr-hot records the inode of every codegen unit object.
2. **Which units changed.** rustc hardlinks reused CGU objects out of the
   incremental cache, so a reused unit keeps its inode and a regenerated
   one gets a new file. No diffing, no hashing.
3. **Patch image.** Only the changed objects are linked by `ld.lld -shared`
   at a fixed address 256 MiB below the PIE's no-ASLR base
   (`0x555555554000`), within `jmp rel32` reach. Every symbol they need
   from the rest of the program is defined in a generated linker script as
   `__ehdr_start + offset`, the base binary's address of that symbol
   (local symbols are matched by their CGU's `FILE` symbol group). Writable
   globals are bound to the base's copy, so there is one copy of each.
   Shared-library imports stay dynamic.
4. **Loader.** `jr-hot run` (the cargo runner) disables ASLR for the test
   process and preloads `loader.so`. Its constructor runs before any of the
   executable's initializers: it maps the image at its link address, applies
   its few dynamic relocations from the map file, writes `jmp rel32` over
   the first 5 bytes of every replaced function in the base, and registers
   the image's `.eh_frame` so panics unwind through patched code.

Fallbacks to a full link (fail open): no base yet, the base binary changed,
more than half the units changed, a TLS access in a changed unit, a symbol
missing from the base, a sub-5-byte function whose bytes changed, any lld
error. Test subprocesses inherit `LD_PRELOAD`, but the loader only acts in
the exact executable (inode, size, mtime) the map was written for.

Differences from Dioxus subsecond: subsecond keeps a process alive and
swaps a jump table that calls go through (`subsecond::call`), so code has to
opt in. Tests start a fresh process every run, so here the patch is applied
before `main` by rewriting function entries, and nothing in the crate
changes.
