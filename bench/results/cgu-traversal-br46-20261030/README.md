# br46 shared per-CGU inline traversal evidence

Portable offline replay:

```sh
python3 validate.py
python3 test_validate.py
```

Five stacks, 25 accepted timed edits, 50 total rows, 30 owned private recorder
states and 30 dedicated exports. Exact negative/type-recovery/test-failure/test-
recovery gates, four expected tests and unique per-edit runtime markers are
replayed from unabridged raw logs. Seed/source/tool identity receipts agree.
The driver hash is pinned to
`29ea709935d751ce13e77995173659006c1e15683e0f0645616d12cd5a3a1665`.

Non-win: candidate median 1.233052 seconds versus baseline 1.137701 seconds.
Other medians: all3on 1.140851, Blaze 0.093685, Blaze LLVM 0.102377 seconds.
No significance, general speedup or default-promotion claim. Native option
`-Zjr-cgu-shared-inline-traversal=yes` remains default-off. Separate profiles
are not speed claims or part of these protected runtime measurements.

`state/` contains the exact thirty private recorder directories. `exports/`
contains ten slim calls from each private br46 home, not a global export.
No caches, targets, binaries or sockets are included. The SHA-256 inventory
covers all retained files, including validator and mutation-test sources.
`seal.py` creates it once and refuses to overwrite it. Semantic mutations
update raw hashes and paired recorder copies, forcing independent gate checks.

Only bundle files and Python's standard library are required for replay, even
outside the repository. Use normal Python, not `python -O`. Absolute paths in
receipts are historical strings, not private file dependencies. `retain.py` and
`launch-runtime46.sh` are historical utilities, NOT replay commands. No native
workloads were run while retaining this evidence.

Full protocol and scope: `docs/cgu-traversal-br46-20261030.md` in the repository.
