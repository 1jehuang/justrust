# Native compiler slots

The opt-in native manager owns only prefixes in `toolchains/managed/` (including
its `staged/` child). Legacy clients never select this namespace, so their
unleased builds cannot race deletion of managed compilers. The manager never
adopts or deletes existing ordinary installations. It never changes global
default policy, project pins, cache directories, or unrelated installations.
IDs must be exact `jr-` IDs with 7 through 40 hexadecimal digits, including
legacy short IDs.

```sh
justrust toolchain slots init jr-aaaaaaa
justrust toolchain slots begin jr-bbbbbbb
# Explicitly select the candidate for the desired correctness workload.
JUSTRUST_TOOLCHAIN=jr-bbbbbbb justrust test
justrust toolchain slots validate jr-bbbbbbb
justrust toolchain slots cleanup
justrust toolchain slots status
```

For the hosted build account, use `justrust remote toolchain-slots` with the same
subcommands. This reuses the authenticated hosted transport and the exact
checksum-pinned client executable, so it needs neither a PATH-installed remote
client nor a new daemon. The operator client must be Linux with the same
architecture as the hosted machine. For example:

```sh
justrust remote toolchain-slots status
```

`init CURRENT` native-installs/reverifies CURRENT and registers it as known-good.
`begin NEXT` native-installs/reverifies CURRENT and NEXT, retaining CURRENT as
last-good. Downloaded candidates remain staged: begin does not promote production.
`validate ACTIVE` is an explicit operator assertion that correctness validation
has passed. It verifies installation but does not run a correctness workload or
change global policy. Only after validation may cleanup delete the previous
registered prefix, and only after usage drains. There is no automatic rollback.

Two occupied slots block a third candidate. To fix forward after a failed
candidate, explicitly discard the failed candidate after its builds drain:

```sh
justrust toolchain slots discard jr-bbbbbbb
justrust toolchain slots begin jr-ccccccc
```

Discard accepts only the unvalidated active candidate, never last-good. It removes
only that managed candidate and resets manager bookkeeping to last-good. The
durable `discarded` receipt records the failed ID so release orchestration can
prepare a fix while global policy still names the failed release. It does not
modify production policy, launch a rollback, or change a project pin.
If usage remains, discard reports `busy` and changes nothing. A successful
discard permits a new candidate without ever retaining a third managed prefix.

## JSON contract

Every successful command prints one JSON object with `schema_version: 1`,
`active` and `last_good` string IDs (null before initialization), `validated`,
`previous` and `discarded` (nullable IDs), and `status`. Status and ordinary lifecycle mutations
use `status: "ok"`. Cleanup uses `"cleaned"`, `"nothing"`, or `"busy"`;
discard uses `"discarded"` or `"busy"`. Busy exits successfully and leaves state
and compilers unchanged, allowing callers to retry after builds drain. Invalid
state, unsafe paths, missing validation, invalid IDs and incorrect lifecycle
requests fail with a nonzero exit status and an explanation on stderr.

## Safety and scope

State is stored at `~/.justrust/toolchains/.slots.json` (under the configured
justrust home). Publication uses a same-directory temporary file, file fsync,
atomic rename, and directory fsync. Lifecycle and install locks serialize the
entire transaction. Compiler selection takes the lifecycle lock before lookup,
then shared per-ID usage locks for the full recorded/unrecorded Cargo lifetime.
Cargo inherits its usage lock so a killed recording parent does not alone permit
deletion while Cargo or its descendants remain alive. Through the build daemon
(`buildd`, hosted agents) the lease follows the same Cargo lifetime: when the
last client goes away before an early answer, buildd stops Cargo as the
client's kill would have and the lease is released; after an early answer
buildd keeps Cargo (and its lease) running so the verdict can replay
(`tests/toolchain_slots.rs::buildd_holds_usage_lease_until_it_stops_orphaned_cargo`). Cleanup/discard try an
exclusive usage lock without waiting and retain a busy compiler.

A validated previous managed compiler stops accepting new managed builds before
cleanup, so selection cannot race deletion. An explicit pin can still use or
reinstall the same ID in its ordinary installation namespace. Those prefixes
are never manager deletion targets. Existing installations and required
bootstrap toolchains are outside the two managed release slots, and are not
silently purged during migration.

Compiler binaries invoked directly inside `managed/` bypass justrust usage
leases. Do not invoke managed paths directly. Older clients and direct users of
ordinary compiler paths are isolated from manager cleanup by namespace. Platforms
without inheritable usage-lease safety refuse deletion.

An interrupted install can leave an unregistered managed candidate. Only an
exact retry may adopt that candidate. A different release refuses to create a
third managed compiler until the interrupted attempt is explicitly resolved.
Published archives are retained independently for recovery. No new background
process or service is introduced.
