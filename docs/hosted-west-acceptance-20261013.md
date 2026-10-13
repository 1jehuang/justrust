# Independent hosted-west acceptance, 2026-10-13

This is functional acceptance, not a performance comparison or a 10x claim.

## Accepted migration and source guard

The migration owner released the build window at 23:00 UTC and reported live
worker `fe38bf38`, from private cloud commit `b01662a`. Local inspection
confirms that commit contains the counted `reclaiming` capacity guard,
following region-aware host management in `fa581a6`.
The accepted own-account host was an M8azn 6xlarge in us-west-2a. The east
machine was stopped and retained with its original volume, not deleted.
The other account's east host remained managed in its original region.
These infrastructure facts are the migration owner's authenticated handoff,
not observations inferred from a client timing or the word "hosted".

The owner also reported region-scoped cost estimates and null for unknown
regions. The shared fleet instance-type default remained unchanged.

## Independent installed-client workflow

Commands were plain `justrust check`, `justrust test`, and
`justrust clippy --all-targets -- -D warnings`, without forced remote,
local routing, a frozen historical client, or changed compiler flags.
The installed client was alpha.16 with SHA-256
`e2bc92690d1e2f0c11f0d7be413fee97e70fb987df0bcfeadfea68e7a4e5f79e`.
Repository HEAD was `82f5d38`, separately from binary identity.

| Command | Exact run ID | Result | Compiled / fresh |
|---|---|---|---|
| check | 20261013-160058612-3890266 | exit 0 | 0 / 56 |
| full tests | 20261013-160059348-3890346 | 360 passed, 0 failed, 2 ignored | 0 / 56 |
| strict all-target lint | 20261013-160102069-3890192 | exit 0, no warnings | 0 / 56 |

Each summary independently verifies the current agent session, exit 0,
24 host cores, and `remote.machine = hosted`. The compiler was the pinned
nightly-2026-10-08. Artifacts were reused; these no-edit acceptance runs do
not measure an edit-loop speedup. Only these three owned records are added
by this evidence commit, not unrelated records from the shared export.

## Capacity failure regression

Offline tests forbid real network calls and use fake AWS plus SQLite.
A scratch reproduction first showed that an AWS DescribeInstances or
StopInstances failure, followed by a failed compensating database write,
left a still-running victim uncounted and allowed a newcomer on retry.
Both cases genuinely failed the no-new-launch assertion.

The owner changed the claim to counted `reclaiming` until a stop was
confirmed. At 22:59:46 UTC, the actual owner source passed seven regional
and reclamation tests plus four independent public-worker outage cases.
The formerly failing compound cases now return 503 on both attempts,
retain the victim's counted claim, and launch zero newcomer machines.
A copied-source candidate also passed, but the acceptance above is based
on the actual source, not substituted code. The root did not edit, stage,
or deploy the owner's cloud source. The live version attribution comes
from the owner's subsequent handoff.

## Scope and follow-up window

The installed-client workflow finished at 23:01:02.740 UTC. The owner then
announced an exclusive, reversible west-host hardware window beginning at
23:01 UTC, so these commands must not be presented as uncontended timings.
No additional
hosted builds, status refreshes, or SSH operations were submitted by this
session after release, so this record is not a claim about the trial host's
later state or hardware.

Owner-published matched warm east/west runs are all within noise, with a
known east-baseline shared-host contention window. No region-only speedup
is established. The last matched stock-local comparison remains below the
10x target. This acceptance does not replace a future uncontended pinned
benchmark or the owner's final hardware-restoration check.
