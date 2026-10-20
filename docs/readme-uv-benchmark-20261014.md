# README uv benchmark methodology

The README table is a focused comparison on uv, not the comprehensive
session-weighted agent workload and not a claim that a code change improved
justrust. Use `bench/versus.py run --synth 1 -n 1` for the broader comparison.

## Results, 2026-10-14

All 27 planned measurements succeeded, plus nine excluded setup/prewarm commands.
Each cell below is the median of three measurements, followed by its observed
minimum and maximum, in seconds.

| Workload | Stock local | justrust local | justrust hosted |
|---|---:|---:|---:|
| Fresh workspace check | 81.16 (76.58–82.67) | 34.54 (32.67–37.18) | 14.33 (14.26–25.33) |
| Fresh resolver test build, no execution | 85.85 (79.33–88.28) | 33.30 (31.73–88.64) | 15.24 (14.32–23.37) |
| Warm resolver body-edit check | 0.956 (0.861–1.181) | 0.849 (0.829–0.883) | 0.891 (0.869–0.973) |

Evidence: [complete receipt](../bench/results/readme-uv-20261014.json),
[host identity](../bench/results/readme-uv-20261014-host-identity.json), and
[exported recordings](../data/runs/xps14/readme-uv-20261014.jsonl).
The initial harness is commit `6c9a6a6`, with resume support in `8f34d5a`.

Local is the XPS 14 with Intel Core Ultra X9 388H and 62.3 GiB RAM. Hosted is
m8azn.6xlarge in us-west-2, 24 AMD EPYC 9R05 cores, 92.5 GiB reported memory.
The laptop's one-minute load at command starts ranged from 11.3 to 26.7.
Stock uses Cargo/rustc 1.98.1. Local justrust uses `jr-ac20de5e7921`, rustc
1.101.0-nightly and its installed jr-cargo. Hosted uses Cargo/rustc 1.99.0.
The client is a frozen justrust 0.1.0-alpha.17 executable, SHA-256
`9813d8938b310c9677b967aeedcefae9b3047292be12e150ef7071e5e5f235ba`.
Local compiler/Cargo hashes were identical across all local recordings.
Hosted compiler/Cargo hashes observed during round one were checked again
after the last sample and were unchanged, not inferred from the client.

Existing shared dependency caches were neither reset nor guaranteed warm.
First-round fills remain in the results, including the 88.64-second local test
build and 25.33-second hosted workspace check. Later fresh checkouts benefit
from those retained caches. This is a sequence of fresh-checkout waits with
persistent caches, not three independently cold-cache trials. All commands use
`--locked`; test-build rows do not run tests. Warm-edit ranges overlap across
stacks, so hundredths of a second do not establish a stable ranking.
Each recorded justrust body edit compiled one unit with 458 fresh units, and
all nine edited source files were verified restored to their pristine hashes.
Published home paths are redacted; the original runtime receipt and full command
logs remain in private scratch.

Validation: all 214 benchmark regression tests passed, along with
`cargo fmt --check` and `git diff --check`. This is focused uv evidence only;
no comprehensive synthetic speedup or default promotion is claimed.

## Reproduce

```sh
python3 bench/readme_uv.py --source /path/to/uv --rounds 3 \
  --work "$HOME/.jcode/scratch/readme-uv-new" \
  --output bench/results/readme-uv-new.json
# Continue the same receipt after an external interruption:
python3 bench/readme_uv.py --resume bench/results/readme-uv-new.json
```

The runner `bench/readme_uv.py` archives uv commit
`44b2e5877ef752e360acc02c7d198ba71ddf948d` into isolated source directories.
Each fresh-check and fresh-test sample gets its own checkout and target state.
Source extraction is outside the timed region, while command startup, source
upload, admission, compilation, and report delivery are inside it.

The three stacks are stock Cargo/rustc on the laptop, justrust's default local
toolchain, and justrust's hosted service. These are product configurations, not
a compiler-only experiment: hardware, compiler versions, flags, and dependency
cache implementations differ and are recorded with the results.

- **Fresh checkout, workspace check:** `check --workspace --locked`, with no
  artifacts in that checkout. Shared dependency caches and Cargo downloads
  persist. This is not an empty-cache or first-ever-install measurement.
- **Fresh checkout, resolver test build:**
  `test -p uv-resolver --no-run --locked`, independently of the workspace-check
  target. It builds test binaries but does not execute tests.
- **Warm edit, resolver check:** `check -p uv-resolver --locked`, following an
  untimed same-command prewarm. Each sample inserts a unique executable line
  into an existing function body, rather than merely appending a comment.
  Sources are restored from their saved pristine bytes afterward.

Samples run sequentially with rotating stack order. Report median wall time
and retain individual samples and load observations. Other agents may be active,
so these are observed end-user waits, not isolated hardware minima. The hosted
route must be confirmed by its run summary, not assumed from `--remote`.
A failed command or silent local fallback cannot supply a successful cell.

The first final-run invocation reached the execution tool's 600-second limit
while round two's stock test build was in progress. Completed samples were
retained unchanged. The interrupted attempt is preserved but excluded from
aggregation, and its replacement starts in a new checkout with an empty target,
not the partially compiled target. The remaining rounds resume the original
plan and rotating order. No successful sample is replaced or selected away.

Before measurement, the hosted root filesystem was full. Its existing EBS
volume was expanded from 200 to 500 GiB, and a quiet-host maintenance reboot let
cloud-init grow the partition and filesystem. Existing files, dependency caches,
instance identity, and isolation policy were preserved. The added storage is
$24/month. The private cloud repository records the maintenance in commit
`038cd7a`, `docs/hosted-root-expansion-20261014.md`.

## Why replace the October 10 table?

The original source in `compiler/FINDINGS.md`, “Remote vs local on uv” (2026-10-10),
measured local justrust, not stock Cargo. Its 77.7-second local fresh-directory
run had only 5/574 dependency-cache hits, while the remote comparison had
574/574 hits. Labeling the former “Standard toolchain” and both “shared cache”
was misleading. The original 1.0-second edit was not backed by a matched
three-stack body-edit experiment. New measurements must not reuse those cells
under more favorable labels.
