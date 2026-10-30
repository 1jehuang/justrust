# br45 native Cargo residency evidence

Offline replay, no native tools or private filesystem inputs:

```sh
python3 validate.py
python3 test_validate.py
```

Seven stacks, 35 accepted timed edits, 70 rows, 50 private owned recorder states
and 50 dedicated bundle-local exports. Every timed daemon edit has the exact
served-by-parked-server raw marker. Source/tool/seed identity receipts agree.

Medians, seconds: owned-off 1.206150, all3on 1.263922, owned-jrcargo 1.177500,
owned-jrdaemon 1.268064, owned-jrdaemon4 1.296977, Blaze 0.097911,
Blaze LLVM 0.104013. No residency win or general performance claim.

Cleanup metadata reports two SIGTERM attempts only. It does not independently
prove historical /proc identity checks or process exits. Daemon4 startup stderr
says `bind ...: server alive`, not `parked at`. Timed raw served proofs, not
startup logs, establish the harness eligibility condition.

`state/` retains all 50 private recorder directories and daemon stdout/stderr,
not targets, caches, executables or sockets. `exports/` contains ten slim calls
from each of the five private br45 homes. No global export was used.
`sha256.json` seals the full retained inventory, including validator/tests.
`seal.py` refuses to overwrite it. Mutation tests independently reject semantic
forgeries with updated raw hashes as well as inventory changes.

`retain.py` and `launch-runtime45.sh` are historical utilities and are NOT replay
commands. Absolute paths in artifacts are provenance strings only.
See `docs/native-cargo-br45-20261030.md` in the repository for the full protocol,
configuration, provenance and scope limits. The standalone validator requires
only this directory. Use normal Python, not `python -O`, for assertion checks.
