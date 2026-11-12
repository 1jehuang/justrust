# Production dogfooding

Production dogfooding means the installed client is byte-identical to the
published artifact selected by the production service, and real builds run on
the normal hosted backend. A source build with the same package version is not
a production release.

## Edit to production

1. Edit on `stable`. Validate with hosted `justrust check`, `justrust test`, and
   `justrust clippy`, plus the applicable benchmarks for performance changes.
   These commands exercise the checkout, not a newly installed CLI. Candidate
   CLI testing must use an explicit candidate path and be labeled as such.
2. Commit and push. Bump the package version and lockfile for a new release,
   commit and push that change, and tag that `stable` revision with `vX`.
3. Release CI verifies the tag/version and requires passing CI for that exact
   revision before building. Linux distribution builds must use hosted. Other
   platforms still use the existing native GitHub runners. Publishing makes
   the release eligible for production selection, so it exposes it to users.
4. Verify that the production endpoint selects that release. Selection can lag
   publication by the service's five-minute cache, or be held with `RELEASE_TAG`.
   Do not bypass a hold or the update kill switch to claim production acceptance.
5. Activate and exercise the selected release:

   ```sh
   python3 scripts/production-dogfood.py --expect v0.1.0-alpha.24 --activate
   ```

   Substitute the intended published tag. The command downloads and verifies
   the release checksum, version, and release-build marker. It performs a real
   hosted smoke test before replacing the PATH-selected executable atomically,
   retains the old executable, and repeats hosted acceptance through the
   installed path. Local compilation and local fallback are not accepted.
6. Run representative real workloads with the installed client, inspect
   `justrust show <id>` for `remote on hosted`, and cite the recorded run IDs.
   A scratch-crate smoke test is not a substitute for those workloads or the
   comprehensive performance gate. No speedup is implied by activation.

To check installed artifact identity without replacing anything:

```sh
python3 scripts/production-dogfood.py --expect v0.1.0-alpha.24
```

This read-only mode checks service selection and exact published bytes, not
hosted execution. `justrust update status` alone is insufficient because package
versions do not identify unpublished source changes.

## Hosted identity and scope

On matching Linux architectures, durable hosted sessions install an immutable,
SHA-256-verified copy of the invoking executable and pin the session to it.
The activation command uses this existing production transport with the exact
published artifact. It does not replace a shared global launcher, change other
accounts, force a development agent, or disable tenant isolation or billing.
It clears `JUSTRUST_*` development overrides except `JUSTRUST_HOME` and sets
required-remote durable transport for its commands. Credentials remain intact.
Existing running commands keep their old binary. Start subsequent commands
through the newly installed path.

The script currently supports Linux x86_64 and aarch64 and requires an existing
signed-in installation. It updates the executable selected by PATH, not every
other copy on disk. Do not use an explicit path to an older Cargo installation
and call that production dogfooding. The standalone helper does not tag, publish,
or automatically release every edit. Candidate promotion remains a separate
workflow and is not implemented by this command.

## Failure and rollback

The helper fails closed on a held/mismatched production release, disabled
updates, invalid archive/checksum, source-build marker, failed test, or a run
record that is not hosted. No executable replacement occurs before initial
hosted acceptance. After replacement, failures remain visible and the helper
does not claim success or silently hide them by switching versions.

The previous executable is retained under
`$JUSTRUST_HOME/production-backups/justrust-<sha256>` (normally
`~/.justrust/production-backups`). The activation command prints the exact path.
An operator can restore that executable to the printed installed path using a
temporary file and atomic rename. Restoring an unpublished source build is an
emergency recovery action, not production dogfooding.

Fleet rollback is distinct from restoring one workstation. An authorized cloud
operator can pin `RELEASE_TAG` to a known-good release or disable automatic
updates in `justrust-cloud`, deploy, and verify the production endpoint. Then
activate the selected known-good release explicitly with this helper. This
helper does not change fleet policy or downgrade other users automatically.
