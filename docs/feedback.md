# Feedback command acceptance, October 14, 2026

`justrust feedback "message"` sends an explicit message to the maintainer inbox.
It sends only that message, not build reports, source, paths, account credentials,
or an install identifier. Anything typed in the message is sent, so do not include
secrets or private information. Telemetry opt-outs also disable feedback.

The command sends synchronously and reports success only for the endpoint's
HTTP 201 response after the durable database insert. Failed requests are not
queued or retried automatically. A connection failure can leave receipt
unconfirmed, so a manual retry can produce a duplicate.

## Live acceptance

- Installed with `cargo install --path . --locked`.
- Installed `justrust feedback --help` shows usage and privacy guidance.
- `JUSTRUST_NO_TELEMETRY=1 justrust feedback "..."` exits unsuccessfully with
  an explicit not-sent explanation.
- Applied the cloud repository's `migrations/2026-10-14-feedback.sql` and
  deployed initial worker version `8779da30-ff85-4724-ac09-df4e60e4c471`, then
  final rate-limit cleanup version `ce11a8c5-bda1-4f79-853c-b7cfbf0a4e23`.
- Sent an explicitly synthetic acceptance message through the installed CLI
  at 07:27 UTC and read its exact text back from production D1, feedback id
  `95882cc4-2132-4747-98f6-db24e4147165`.
- Reinstalled the final frozen client and verified a second explicit synthetic
  message against the final deployment at 07:34 UTC, feedback id
  `48a009fe-60d7-4cf7-8e81-d7e5184e630a`.

## Regression checks

- Hosted check: `20261014-003129820-2844565`, exit 0.
- Hosted library suite: `20261014-003134142-2845026`, 378 passed, 3 ignored.
- Hosted CLI integration: `20261014-003147829-2846899`, 2 passed.
- Hosted Clippy: `20261014-003150578-2844481`, no warnings.
- Local focused unit tests: `20261014-003134577-2845054`, 2 passed.
- Local integration Clippy: `20261014-003139307-2845711`, no warnings.
  These two local runs followed a hosted connection timeout.
- Cloud `npm test`: 212 JavaScript tests and 28 Python tests passed.

Exported summaries are in `data/runs/xps14/feedback-2026-10.jsonl`, isolated
from unrelated pending history changes. No compile-time speedup is claimed.

Maintainer retrieval is documented in the private `justrust-cloud` repository.
Feedback is stored separately from expiring anonymous usage telemetry and has
no public read endpoint.
