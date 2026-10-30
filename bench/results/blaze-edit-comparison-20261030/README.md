# Focused Blaze comparison evidence

See `docs/blaze-edit-comparison-20261030.md` for exact commands, methodology,
all sample durations and limitations. This is a single pinned library-check edit,
not a broad benchmark, executable correctness test or default-promotion gate.

- `complete/`: full three-stack run, three unique edits per stack, passing
  deliberate type-error/recovery gates, stable executable/driver identities.
- `failed-long-socket-path/`: preserved first attempt. Blaze failed before
  measurement because a generated Unix socket path exceeded the platform limit.
  Its owned-only partial observations are not merged with the complete run.
- `smoke.*`: earlier compatibility smoke, excluded from warm medians.
- `premeasurement-manifest.json`: all 63 archived tracked source file hashes and
  pinned Blaze program hashes. All source bytes were rechecked after restoration.
- `daemon-isolation.json`, `daemon-status.*`, `daemon-stop.*`: postmeasurement
  private store/daemon provenance and cleanup. Only this benchmark daemon stopped.
- `evidence-sha256.json`: hashes of the initial 107 retained raw files, not a
  recursive manifest of subsequently added documentation/audit files.
- `regression-and-export.log`: 538 full bench tests passed and 12 owned calls
  exported, including expected errors.
- `validate.py`: replayable audit of matched source hashes, all gates, successful
  sample coverage, identities, raw file hashes and exported run IDs/statuses.
  Run `python3 bench/results/blaze-edit-comparison-20261030/validate.py` from
  any directory. It writes `validation.json` and fails closed on a mismatch.

Observed warm medians: owned-off 1.015603 s, owned-on 1.015780 s, Blaze 0.114129 s.
All samples have unknown contamination, and bounded wait polling overhead is
included. These numbers do not establish a causal retention improvement or
interchangeable compiler correctness. No production defaults were changed.
