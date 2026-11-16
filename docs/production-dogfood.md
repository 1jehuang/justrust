# Production dogfooding

Production dogfooding means the installed client is byte-identical to the
published artifact selected by the production service, and real builds run on
the normal hosted backend. A source build with the same package version is not
a production release.

## Edit to production

1. Bump the package version and lockfile on `stable`, then commit and push the
   intended source. Build/package sanity precedes publication, not correctness
   tests or benchmark acceptance.
2. Tag that exact source revision with `vX`. Release CI verifies tag/version,
   builds and packages the binaries, checks identity/checksums and runtime loader
   sanity, then publishes. It does not wait for passing CI. Linux distribution
   builds use hosted. Other platforms use native GitHub runners.
3. Publication exposes the release to users. Every customer uses the same shared
   production default. There is no canary or per-customer test selection.
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

   Activation also repoints any `cargo` link on PATH that proxies to a
   different justrust executable (for example `~/.local/bin/cargo` still
   pointing at a source build in `~/.cargo/bin`), and verification without
   `--activate` fails while one exists. Two client builds in use at once each
   install themselves as the hosted agent and start their own sync daemon, so
   they keep replacing each other's agent and restarting the daemon.
6. Run representative real workloads with the installed client, inspect
   `justrust show <id>` for `remote on hosted`, and cite the recorded run IDs.
   A scratch-crate smoke test is not a substitute for those workloads or the
   comprehensive performance comparison. Tests and measurements happen after
   publication and shared production activation. No speedup is implied by activation.

## Automated post-publication client suite

`.github/workflows/client-production-tests.yml` is a reusable workflow called by
Release only after GitHub release creation succeeds. Its caller must depend on
both `preflight` and `release`, and pass the exact `preflight` tag and full SHA.
It intentionally does not infer the tag's source from `workflow_run.head_sha`,
which can identify the dispatch branch instead of the released tag.

The job checks out the exact SHA and verifies the tag still points there. It
waits up to ten minutes for the normal production endpoint to select that tag,
without bypassing `RELEASE_TAG` holds or disabled updates. It downloads the
published checksum-verified executable into a temporary PATH directory and
checks version/release identity. No unpublished source bootstrap is installed.
It then uses `production-dogfood.py --activate` for hosted smoke verification
and runs the full client Rust suite against the tagged checkout using:

```sh
justrust test --remote --locked
```

Before and after the suite, the helper verifies the installed bytes still match
the selected production release. A successful hosted run receipt is required,
and available receipts are uploaded as workflow artifacts. Full test output is
visible in the job logs. This job needs the normal `JCODE_API_KEY` secret and
preserves hosted billing and tenant isolation. It covers Linux x86_64 full-suite
execution, not every native platform integration or performance workload.
Failure is reported after publication, not converted into a publication gate
or hidden with a development binary. Benchmark comparisons and rollback remain
explicit post-activation operations.

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
or automatically release every edit. Shared production activation precedes
correctness and performance acceptance and is not implemented by this helper.

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
Existing installations do not currently support automatic downgrade when an
operator pins an older `RELEASE_TAG`. Verify selection and explicitly activate
the known-good published client where a downgrade is required. Do not describe
that operator pin as a completed fleet-wide client rollback.
