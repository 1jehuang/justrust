# Discord release announcements

Justrust uses the same webhook approach as Jcode's release channel. The notifier
was adapted from Jcode's MIT-licensed `scripts/post_discord_release.py` and tests.
It labels messages `justrust`, includes release notes and a release link, disables
mentions, and records a hidden per-tag marker in the GitHub release body to avoid
duplicate announcements on normal retries. A post followed by a marker-write
failure needs manual inspection before retrying, since exactly-once delivery
across GitHub and Discord is not guaranteed.

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
