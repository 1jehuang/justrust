# Fast published-client release loop

Routine client releases should use the same executable users receive, not a
private source build installed on the developer's PATH.

## One command from committed source

After committing and pushing the client and its Cargo version on `stable`:

```bash
python3 scripts/release-client.py --tag v0.1.0-alpha.29
python3 scripts/release-client.py --tag v0.1.0-alpha.29 --execute
```

The first command is a side-effect-free plan. The second:

1. Resolves one immutable committed source revision, checks stable ancestry and
   the committed Cargo version, and refuses a tag naming different source.
2. Pushes the tag once. This triggers the existing release workflow, rather than
   dispatching a second build of the same source.
3. Waits for the shared production endpoint to select the exact published tag.
   It does **not** wait for the full workflow's post-publication tests to finish.
4. Downloads checksum-verified production bytes using `production-dogfood.py`,
   verifies the release marker, runs the hosted smoke, backs up the old client,
   and atomically activates the same published executable on PATH.
5. Runs the post-activation hosted smoke. A failure stays visible, with the old
   executable retained in `~/.justrust/production-backups`. It does not silently
   claim success or replace the failure with a private source build.

`--ref <commit>` selects an earlier pushed stable revision. Uncommitted files
are never packaged, even when the shared checkout contains other agents' work.
The default publication deadline is 30 minutes, configurable with `--timeout`.
An existing remote tag is reused without force-pushing or duplicate dispatch.
For an actually failed build, inspect the reported Actions run and explicitly
rerun that workflow before retrying the command. Never move a published tag.

The local activation helper currently supports Linux x86_64/aarch64. It refuses
unsupported platforms before creating or pushing a tag.

## Keep the release critical path small

Before publication: committed-source/tag/version checks, frozen compiler-policy
selection and provenance, platform builds, package/checksum and executable sanity.
After publication: full published-client hosted regression and broader CI.
Linux runners download a reviewed, SHA256-pinned published bootstrap instead of
compiling a bootstrap client on every release. Compatibility builds cache their
dependencies and smoke-test native executable versions and release markers.
Artifacts are published independently per platform. Preflight creates a shared
draft. Each platform uploads its checked archive and compiler provenance, then
its checksum last. The first ready platform publishes the version, and slower
platforms append their own assets later. Published bytes are never overwritten:
identical retries are verified and reused, conflicting bytes fail closed.
The updater selects a complete archive/checksum pair for the requested target,
keeping the previous complete release while that target is still building.
Repositories with GitHub's immutable-release setting enabled cannot append
platforms after publication. The publisher fails clearly rather than weakening
that repository setting or replacing assets.
Compiler performance experiments are not prerequisites for an unrelated client
reporting fix. Changing the default compiler still needs its own acceptance.

Stable branch CI and same-PR updates coalesce so obsolete commits do not consume
all runner slots. Tag checks remain independent. Platform builds and immutable
artifacts still have real costs, but a queued Mac or Windows build no longer
blocks publication and local activation of an already checked Linux executable.
The final all-platform job reports completeness and starts full published-client
acceptance. It is not the publication gate.

The shared production selector may briefly cache the previous release, or be
held by the operator rollout controls. The helper waits for shared selection
instead of bypassing it or pretending a private client is production. It fails
closed on timeout or a moved tag. Daily client auto-update is not used to keep
the developer's test client current.

## Validation

```bash
python3 -m unittest discover -s scripts -p test_release_client.py -v
python3 -m unittest discover -s scripts -p test_client_ci_concurrency.py -v
python3 -m unittest discover -s scripts -p test_production_dogfood.py -v
python3 ci/test-publish-platform-release.py
python3 -m unittest discover -s scripts -p test_release_order.py -v
```

These are offline safety/regression checks, not evidence of a faster completed
release. Measure a real tagged run's publication time separately from its
post-publication acceptance time before claiming an end-to-end speedup.
