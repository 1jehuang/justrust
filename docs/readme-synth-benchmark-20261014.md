# Comprehensive README benchmark attempt, October 14, 2026

Command:

```sh
JUSTRUST=/path/to/frozen/justrust python3 bench/versus.py run \
  --synth 1 -n 1 --only stock,justrust --remote --no-report
# Resume the same receipt after an interrupted server process:
JUSTRUST=/path/to/frozen/justrust python3 bench/versus.py run \
  --synth 1 -n 1 --only stock,justrust --remote --no-report --resume
```

The frozen client SHA-256 is
`9813d8938b310c9677b967aeedcefae9b3047292be12e150ef7071e5e5f235ba`.
The run observed all 40 session-weighted points, with one repetition per point.
All 40 points have stock-local and justrust-local wall samples. However, the
comprehensive gate is **blocked**, not passed:

- 25 points contain hosted measurement errors, including initial prebuild
  failures and forced-remote commands that fell back locally after sync failed.
- Only seven retained remote samples explicitly identify a hosted machine.
  Other samples in the remote stack have no hosted identity and cannot be used
  as hosted speed evidence.
- Samples were marked noisy under concurrent load. The gate also flags missing
  or inconsistent exit/compile/test outcome provenance for point 37.
- Server reloads interrupted the run twice. Resume used the same frozen client
  and retained the completed points. An earlier attempt using a changing
  installed binary was rejected and is not speed evidence.

[Full completed receipt](../bench/results/versus-synth1-20261014-040027-8e60305db025-dirty.json)
and [53 exported path-redacted recordings](../data/runs/xps14/readme-synth1-20261014.jsonl)
are retained. Two retained sample IDs have no exported detailed recording:
`20261014-024338340-47796369` and `20261014-034820257-1928085242`.
Their wall samples exist in the receipt, but do not support detailed phase claims.

No aggregate speedup, comprehensive performance improvement, or default
promotion is claimed from this attempt. A follow-up needs controlled load,
complete valid outcomes, and confirmed hosted execution for every remote point.
The README instead uses the separately completed [three-round pinned uv
comparison](readme-uv-benchmark-20261014.md), explicitly scoped to those workloads.
Its ratios divide the unrounded stock median by each unrounded justrust median.
Warm-edit sample ranges overlap, so those ratios do not prove a stable ranking.
