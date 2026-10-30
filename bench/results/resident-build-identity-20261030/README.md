# Resident engine build-identity acceptance

This bundle tests only `toolchain/resident/crates/ra-jit/build.rs`. It does not certify the resident frontend, establish a performance improvement, or enable any shipping behavior.

The exact build-script snapshot SHA256 is `7b2c813f9fbba24c6d84ff8dc79f9192f3d57577bf2f656bb41f768153bab49b`. `acceptance.json` records the compiler executable SHA256 and nine process observations, with full stdout/stderr retained. The compiled test executable is deliberately not committed.

The native fixture gate compiled the script with the pinned Rust nightly-2026-10-27 compiler, then checked absent workspace identification, the independent length-delimited SHA256 oracle, target/feature/debug/source invalidation, explicit version-label handling, and fail-closed behavior for a missing declared source input. All nine observations passed. Missing workspace metadata remains explicitly unidentified, never a silently certified build.

The source identity is salted with `justrust-resident-v1` plus the exact Blaze origin before hashing declared source, target, features and debug configuration. The enclosing justrust Git HEAD is not reported as an upstream Blaze revision. A cosmetic explicit version label does not change the source key. The native entry must independently reject unidentified operational builds.

Replay against the current script into a fresh private output directory:

```sh
python3 bench/resident_build_identity_accept.py \
  --rustc /absolute/path/to/matching/rustc \
  --source bench/results/resident-build-identity-20261030/build-script.rs \
  --output /absolute/new/private/output
PYTHONPATH=bench python3 -m unittest -v test_resident_build_identity_accept
```

The three Python regression tests use no native compiler subprocesses. They check directive precedence, an independently assembled salted key, and rejection of optimized Python that would disable acceptance assertions. Native fixture validation is a separate explicit command. This is not a claim of hermetic compilation or resistance to a malicious same-user process modifying source during a build.
