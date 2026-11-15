# Frozen hybrid receipt archives, November 15, 2026

- `hybrid-resident-v5-final-20261115.tar.gz`: original `a5` automatic and `p4` pinned reports, all captured foreground stdout/stderr and exact recorded build outputs, available native lifecycle logs, and original command completion logs. SHA-256: `fa8ac9a7e749fc899407e76f783439ad699ccb1a2fb73ef63b87aa948d57b65d`.
- `hybrid-resident-v5-a4-failed-check-policy-20261115.tar.gz`: separate rejected `a4` evidence, before the client explicitly skipped unsupported automatic check preparation. SHA-256: `b2f4411d602bd1199972c57463267ac0fdd7866a77d69c4c2e3d7df266990775`.

Each archive contains a generated `RECEIPTS.json` with original relative paths, byte counts, and SHA-256 digests for every captured file. Source trees, target artifacts, environment dumps, sockets and credentials are excluded. Capture uses `bench/hybrid_workload_archive.py`, whose tests verify exclusion and reject likely secret-bearing receipts or symlinks.

Read the original JSON reports rather than treating this README as measurement evidence. `a5` uses frozen release client v6 and native engine v5. `p4` uses release client v5 and the same native engine v5. Exact binary identities and child environment allowlists are in the reports. Both compare against the frozen published shipping client.

The automatic check row requires one exact positive unsupported-command skip receipt, no queued preparation, an unchanged prewarm log hash and byte count, and no test or program effects. All supported automatic preparations retain positive completion requirements. Error, recovery, invalidation, nonce, exactly-once effect, route and cleanup gates remain strict.

These are focused correctness and latency diagnostics, not the comprehensive session-weighted promotion gate. `p4` may overlap the coordinator's client build. Its contemporaneous load and CPU proxy are preserved in each row's `noise` object, which explicitly marks contamination unknown because resident daemon CPU is not waited child CPU. Namespace PSS values are outside-timing snapshots, not peaks or total-machine memory. Native `1M` is a soft worker LRU budget, not a total RSS limit or a heap restoration mechanism. No managed-build speed claim or default promotion follows from these archives.
