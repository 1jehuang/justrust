# Async client publication validation, 2026-11-16

- Immutable release: `v0.1.0-alpha.31`, source `a33564ba56e3371097ec2bc3add5e1414acd03b5`.
- Actions run `43894554239`: all five platforms completed compilation, executable sanity checks, packaging and artifact upload. Each publication step failed because GitHub's by-tag REST route omitted the draft.
- Fix `c376106a6` resolves the authenticated release database ID before fetching release metadata. All 24 offline publisher tests passed. No tag was moved and no client was rebuilt during recovery.
- Recovered the exact uploaded Actions artifacts through the append-only publisher. Archive and provenance upload precede checksum readiness. First platform publication preserves global latest with `--latest=false`.
- At 15:31 UTC, production selected alpha31 for ARM Linux while Windows and both macOS targets still selected alpha30. Thus an incomplete release did not send those clients to missing downloads.
- After all 15 assets were present, the completeness helper promoted global latest to alpha31. All five production selectors returned alpha31 at 15:37 UTC.
- Activated the checksum-verified published x86 Linux binary, SHA256 `9cb4dd709b19039a6956966bb10bb70d914cf2300a7796825b43351532ecf2fc`. Hosted pre/post activation smoke passed, final receipt `20261116-073505322-1063763`. Previous executable is backed up by the activation helper.
- Exact-source full hosted acceptance is separate from those smoke checks. Manual Actions dispatch was unavailable because the reusable workflow is not registered on the repository default branch. A full suite was therefore launched with the published alpha31 CLI against an archive of the immutable source instead. Do not interpret the original Actions run as successful: its publication failures remain recorded.

- The full immutable-source suite failed: 1,365 tests passed, four remote-agent integration fixtures failed, 20 ignored (run `20261116-073520122-1065092`). All four failures reproduced in a serial remote-agent-only rerun (`20261116-073900334-1078056`), so these are not being dismissed as parallel-test flakes. The remaining suites are being exercised separately while fixture isolation is investigated. This is not a green full-suite acceptance claim.

The fixed workflow publishes future checked platforms directly from each matrix job. The original alpha31 run required artifact-only recovery, so it does not establish unattended CI publication timing after the draft-lookup fix.
