# Independent optimizer composition acceptance

The first publication attempt for capture receipts `962fd27b9` was rejected non-fast-forward because the independent optimizer commit `1f6924cf6` had advanced origin from the common `be5f1e122` baseline. Both changes were preserved and merged non-destructively as `8e210f7aa9804af959bda62e98c6da8212b42bea`. No reset, force push or reuse of the earlier capture-only full verdict was used.

| Composed check | Run | Result |
| --- | --- | --- |
| Forced-hosted check | `20261018-220649444-1406625` | Passed |
| Forced-hosted optimizer, closures, closure authority and graph suites | `20261018-220655532-1406416` | 25 passed |
| Full local JOBS4 no-fail-fast suite | `20261018-220737219-1427632` | 3,439 passed, zero failed/ignored/filtered, 192 binaries |
| Forced-hosted all-target clippy with `-D warnings` | `20261018-221400264-1420968` | Passed |

Formatting passed before the full source freeze. No isolated-retry warning appeared. The full test run used the default CLI behavior of this composed source; the optimizer's separate off/on differential coverage is part of the optimizer suite. The 383.0-second full-run duration was heavily contended and is not speed evidence or justification for a default performance promotion.

The four owned redacted records are in `optimizer-composition.jsonl`. Earlier capture-only acceptance remains separately recorded in `CENTRAL-ACCEPTANCE.md` and `central-acceptance.jsonl`.

Follow-up review remains open for precise optimizer scratch and retained-allocation accounting and the scope of default-promotion measurements. Passing semantic tests and graph reachability checks do not by themselves establish complete accounting coverage. No new baseline exemption or budget increase is authorized by this receipt. Full Rust support remains incomplete and the unchanged full-program corpus remains 3/20.
