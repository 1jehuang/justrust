# Read-only registry acceptance archives

These are unchanged crates.io `.crate` archives copied from the existing Cargo
cache, not rewritten or path-patched implementations. Tests unpack into isolated
scratch cache homes. No registry network fallback is permitted.

| Package | SHA-256 | Retained license |
| --- | --- | --- |
| utf8-width 0.1.8 | 1292c0d970b54115d14f2492fe0170adf21d68a1de108eebc51c1df4f346a091 | MIT, LICENSE inside archive |
| nix-base32 0.2.0 | d2628953ed836273ee4262e3708a8ef63ca38bd8a922070626eef7f9e5d8d536 | Apache-2.0, LICENSE-APACHE inside archive |
| time-core 0.1.2 | ef927ca75afb808a4d64dd374f00a2adf8d0fcff8e7b184af886c3c87ec4a3f3 | MIT OR Apache-2.0, both license files inside archive |

Original authorship, source metadata, manifests, documentation, and licensing
remain intact inside each archive. They are external test inputs only, never
compiler internals. The first package is the requested native runtime gate. The
other two are diagnostic boundary probes, not claims of library compatibility.
