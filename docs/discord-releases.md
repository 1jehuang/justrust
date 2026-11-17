# Discord release announcements

Justrust uses the same webhook approach as Jcode's release channel. The notifier
was adapted from Jcode's MIT-licensed `scripts/post_discord_release.py` and tests.
It labels messages `justrust`, includes concise release highlights and a release
link, disables mentions, and records a hidden per-tag marker in the GitHub release body to avoid
duplicate announcements on normal retries. A post followed by a marker-write
failure needs manual inspection before retrying, since exactly-once delivery
across GitHub and Discord is not guaranteed.

## Highlights

An authored `## Highlights` section takes precedence over the rest of the release
body. When GitHub auto-generated notes contain only a comparison link, the
notifier reads that exact release comparison and selects up to five shipped
client changes from commit subjects and their actual changed files. It omits
release-version bookkeeping, test/benchmark/documentation-only changes, and the
unpublished compiler experiments under `toolchain/`. It makes no inferred
performance claims. If no client highlights can be found, announcement fails
with a request for authored highlights rather than publishing another bare link.

Hidden availability and announcement markers are never copied into Discord.
Messages remain within Discord's length limit and always retain the release link.

Preview the actual announcement without webhook credentials or public writes:

```sh
GH_TOKEN="$(gh auth token)" python3 scripts/post_discord_release.py \
  --repository 1jehuang/justrust --tag v0.1.0-alpha.33 --preview
```

## Updating an existing announcement

Supply the existing Discord message ID in the workflow's optional `message_id`
input. The notifier fetches that webhook message and verifies its product,
release tag and release URL before editing it in place. It never creates a new
message in this mode, and it still disables mentions. Successful posts/edits also
record a hidden message-ID marker alongside the original per-tag marker so later
repairs do not require searching the workflow logs. For older posts, the original
workflow log contains the message ID printed after posting.

The CLI equivalent is `--edit-message-id ID`. A normal retry without that option
continues to skip already-announced tags.

## Activation

1. Land `.github/workflows/discord-release.yml` and both `scripts/*discord_release.py`
   files on the repository's default branch. The clean-room branch alone does not
   activate production release notifications.
2. Add the existing release channel webhook as the Actions repository secret
   `DISCORD_RELEASE_WEBHOOK` in `1jehuang/justrust`. GitHub does not allow reading
   an existing secret back from Jcode. Retrieve the webhook through Discord's
   channel integrations instead. Do not paste it into source, logs, or chat.
3. Run the workflow manually with an existing release tag when ready to send a
   real announcement. This publishes a Discord message, not a dry run.

Published-release events cover external publication. Successful tag-push runs
of the existing `Release` workflow cover publication using `GITHUB_TOKEN`, whose
release events GitHub suppresses. Manual runs of the publisher on a branch need
an explicit manual announcement with the released tag afterward. Draft releases
are rejected. Alpha and prerelease versions are intentionally included.

Test locally without sending anything:

```sh
python3 -m unittest -v scripts/test_post_discord_release.py
```

No compiler builds or stable-client installation are required for this integration.
