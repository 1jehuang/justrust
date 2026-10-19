# Native mirrored-target garbage collection

Agent upgrades no longer forget mirrored-root activity. Each authenticated SSH
agent's successful `Synced` message enrolls the exact canonical source root in
`$JUSTRUST_HOME/remote/evict-registry/` (normally `~/.justrust/remote/`). The private
0700 namespace is versioned and scoped to the effective OS UID and canonical
agent prefix. Its 0600 receipts bind UID, prefix scope, canonical root, device,
and inode. No command imports roots from a filesystem scan. These receipts rely
on the existing SSH/account host isolation and OS ownership boundary, not on
sharing one Unix identity between mutually untrusted tenants.

Only `<registered-root>/target` is eligible. Symlinks, changed root identities,
foreign owners, target/root filesystem mismatches, private `.justrust`/`.cargo`
paths, active leases, process cwd/open descriptors, and activity within two hours
are excluded. External `CARGO_TARGET_DIR` paths are not candidates. Manifestless
source roots are supported, including monorepos whose Cargo projects are nested.

Every run holds a shared per-root flock. Legacy 0664 lock files are tightened to
0600 through the same verified open owned regular single-link inode, without
unlinking, recreating, truncating, or releasing another process's existing flock.
No-follow opens and canonical owned safe-parent checks still reject aliases,
foreign ownership, hardlinks, and unsafe directories. Acquisition failures report
the exact path and underlying OS error, distinct from ordinary lock contention.
Enrollment and build start/end update
and fsync receipt metadata. A persistent uncertainty marker is set before a run
starts and cleared only under an exclusive flock after all agents release their
build locks. A crash or failed end update therefore stays protected across an
upgrade, even if its start timestamp is old. A later successful, exclusively
finalized build clears that marker and starts a fresh two-hour grace period.
Uncertain markers are deliberately not cleared by a process scan or a timeout.

A collector reloads receipts on every pass and rechecks activity, leases, and
process use after obtaining the exclusive per-root lock, immediately before
revalidating and deleting the target. Recording failures refuse the affected
run before it starts, rather than silently allowing it to bypass protection.
They do not terminate unrelated routes or the shared transport.

## Explicit maintenance

Run the native command **on the build host**, using the same Unix account,
`JUSTRUST_HOME`, and agent prefix as the authenticated agent:

```sh
/path/to/installed/justrust remote gc
/path/to/installed/justrust remote gc --apply
```

For an agent started with a nonempty prefix, pass its exact canonical prefix:

```sh
/path/to/installed/justrust remote gc --prefix /canonical/agent/prefix
```

`remote gc` itself never connects over SSH, mirrors sources, builds a project,
or enables automatic deletion. Use the existing authenticated `remote ssh`
command to invoke that native binary on a hosted machine. The default is a dry
plan regardless of `JUSTRUST_REMOTE_EVICT`; `--apply` is explicit authorization
for one collection pass. Automatic agent collection still defaults to `dry`.
There is no flag to reduce the fixed two-hour grace or enroll arbitrary roots.

Collection is bounded by disk watermarks: trigger below 15% free or 50 GiB
(with the existing small-filesystem cap), then plan least-recently-active targets
toward the 25% high watermark. Live, recent, invalid, and unknown roots are never
fallback deletion candidates when the eligible plan cannot satisfy that deficit.
No collection occurs when the measured filesystem is above its low watermark.

Diagnostics include known/rejected receipt counts, exclusion reasons, exact
allocated tree bytes, free bytes before/after, and signed observed free-space
delta. Allocated tree bytes are **not** claimed as physical reclaimed bytes:
hardlinks can retain storage and concurrent activity can change filesystem free
space. Actual deletion failures produce a nonzero explicit command result.

## Historical mirrors

Mirrors created before durable receipts exist are intentionally unknown. They
are not discovered by scanning arbitrary directories and are not retroactively
enrolled by the maintenance command. A normal authenticated source sync enrolls
them with fresh activity and the full two-hour grace. Do not manufacture receipts,
backdate activity, clear uncertainty markers, lower grace, or delete active
targets to force immediate capacity recovery. Dependency-artifact LRU pruning is
separate and does not confer authority over mirrored source or target trees.
