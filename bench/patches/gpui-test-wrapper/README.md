# GPUI test-wrapper replay

This project-specific fixture removes a redundant parse of the **generated** outer test function. The candidate still parses and validates the input function. It emits the wrapper directly, preserving outer and inner attribute placement. It does not cache macro invocations or change any compiler default.

## Source and scope

- Patch base: `1jehuang/gpui` commit `be875c56343dcbd45127a9805781a9e128057dd7`, `crates/gpui_macros`. The crate declares Apache-2.0. The adjacent `LICENSE-APACHE` is the pinned repository's license text, including its Zed Industries copyright notice. Its `UPSTREAM` records `zed-industries/zed` commit `7960b2a7c9568e90fbe0727332149e5b2a5fd57a`. `candidate.patch` contains the fixture's modifications, not an upstream or deployed release.
- Replay inputs: `1jehuang/jcode-desktop` commit `7d38fb72a0beadbca366da1104c915689cc0fd8e`. Its lockfile pins GPUI `1cfbc5d35c9865930db3287a5fe15dd95d16f15c`, whose `crates/gpui_macros` tree matches the patch base. Desktop sources are read from your own Git object store and are not distributed here.

## Reproduce

Prerequisites: Linux, Python 3.9+, Git, GNU patch, read access to the two named commits, and a **held Cargo target** containing `lib-gpui_macros.json` plus its compiled host dependencies. Use the same real rustc binary that built those dependencies. Incompatible rlibs are rejected by rustc. The probe uses `RUSTC_BOOTSTRAP=1` for `Span::eq` in its subprocess only.

From the justrust root, with `RUSTC_BIN` and `GPUI_FINGERPRINT` set to those absolute paths:

```sh
work=$(mktemp -d "${JCODE_SCRATCH_DIR:-/tmp}/gpui-replay.XXXXXX")
python3 bench/patches/gpui-test-wrapper/replay.py \
  --work-dir "$work/trial" --desktop "$HOME/jcode-desktop" --gpui "$HOME/gpui" \
  --rustc "$RUSTC_BIN" --fingerprint "$GPUI_FINGERPRINT"
```

The original experiment and packaged replay were validated with the following held artifacts. On another machine substitute your matching toolchain, target and repositories. A rustup *proxy* is not a pinned binary.

```sh
prefix="$HOME/.jcode/scratch/desktop-macro-profile.AeD2/gpui-replay/prefix/usr"
export LD_LIBRARY_PATH="$prefix/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
export RUSTC_BIN="$prefix/bin/rustc"
export GPUI_FINGERPRINT="$HOME/jcode-desktop/target/debug/.fingerprint/gpui_macros-d0e819ec92c8f4e2/lib-gpui_macros.json"
```

The harness copies immutable Git blobs into a **new** scratch directory, applies the patch there and invokes rustc on the isolated fixture. It never changes either supplied checkout. It alternates control/candidate order by call; set `REPLAY_REVERSE=1` on alternate trials in separate fresh work directories. Reported phase totals include bridge and allocator work inside each operation. This is a macro-only replay, **not** a whole-edit benchmark.

A pass requires exact output token kind, value, punctuation spacing, delimiter, token span, and group open/close spans for 724 held functions and three edge cases. Five invalid invocations must produce identical error tokens; five actual failing attribute invocations must produce byte-identical rustc diagnostic JSON with the expected error count. These checks cover this corpus, not all possible Rust inputs.

## Account-private raw evidence

`bench/gpui-wrapper-724-20261120-db8c48b4e737` contains the raw four-round replay, negative diagnostics and the earlier matched whole-edit pairs from a **different** Desktop snapshot (817 GPUI tests). No raw logs or private project sources are checked in. Retrieve the archive and verify it before opening:

```sh
justrust records show bench gpui-wrapper-724-20261120-db8c48b4e737 \
  | jq -r '.data.archive.bytes_b64' | base64 -d > "$work/evidence.tar.gz"
printf '%s  %s\n' db8c48b4e73708e550a5dc696dc78094ca0f8323da553e9ce2fa1b9772ab97d0 "$work/evidence.tar.gz" | sha256sum -c -
```

Median baseline generated-wrapper parse was 585.72 ms across the 724 calls; macro-only replay improved from 1232.80 to 538.81 ms. The separate twelve-pair whole-edit experiment had a median paired saving of 540.11 ms. Neither result establishes a published compiler speedup.
