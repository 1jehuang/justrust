# v46 frozen source-investigation release

Disposition: NOT PROMOTED. No production Rust changes. No wire change and no schema bump.
Working-tree deliverable: docs/resident-block-identity.md only.

Preserved:
- original/: seven complete authorized Rust source files before any editing
- candidate/: the same seven Rust files, byte-identical, and the new document
- candidate.patch: git-style patch adding only the document
- original.sha256 and candidate.sha256: sorted complete source manifests
- rustfmt-check.txt: empty, rustfmt original three assigned files exit 0
- diffcheck.txt: empty, no whitespace diagnostics; no-index exit 1 denotes added doc

No builds, tests, native jobs, installations, commits, or pushes were run.
No fixtures were skipped or assertions changed. No native correctness or speed claim is made.

Established:
- body_owner.module is not the immediate allocated lexical parent for nested blocks
- parent-only recursive wire can represent the exact finite parent chain without body-owner recursion
- macro AstId subcodec does not introduce ModuleId edges
- actual schema namespace is ra-jit/src/frozen.rs key_of version 6, with derived impl/langitem keys
- stale tuple validation needs authoritative source/cfg/macro/semantic membership, not just constructor keys
- exact scope discovery must preserve lang-item gates, path failures, resolver-driven destructuring,
  sequential macro grammar and signature/closure parameter traversal differences
- existing served() panics on decoder error, so fail-open promotion needs an explicit outer fallback

Implementation status:
Canonical constructor and scope provider are not implemented. Scope-only shared semantic frontend is
feasible in principle, but exact traversal equivalence and tracked membership are not proven here.
The source document records precise factoring paths and strict original/fullwalk oracle requirements.
Root coordinator owns acceptance and any subsequent expanded implementation or native validation.
