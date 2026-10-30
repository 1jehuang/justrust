# Fresh source scan bulk manifest experiment

The scan still runs fresh Git discovery and fresh `symlink_metadata` for every candidate. It does not rely on watcher silence. This change collects observed `(path, stamp)` pairs and bulk-builds the sorted manifest, instead of inserting each observation into a BTreeMap individually. It also reuses the joined absolute path for metadata and mode extraction.

The map builder is checked against repeated insertion on all 1,555 sequences of length zero through four over two keys and three stamps, including UNKNOWN, differing timestamps, sizes, and modes. This explicitly checks duplicate-key last-observation behavior. The standard library implementation uses stable sorting before bulk construction. Oversized files still accumulate in original discovery order, outside map construction. Existing lossy non-UTF-8 Git-path decoding is unchanged.

Fresh-scan regressions exercise tracked deletion, replacement with a dangling symlink, executable mode changes, source size changes, growth beyond the transfer cap, and recovery below the cap. Existing ignored-lockfile and fallback traversal tests remain gates.

## Focused measurement

Run `justrust test --local --lib remote_sync::tests::scan_phase_diagnostic -- --ignored --nocapture`. Local execution is required to inspect the actual local vendored source tree rather than the hosted mirror. Set `JUSTRUST_SCAN_BENCH_ROOT` to inspect another tree.

The diagnostic captures one fresh observation vector and times both the old repeated-insertion builder and the production bulk builder against those exact same rows, seven samples each. Cloning happens outside timing, equality is asserted for every pair, and complete fresh scan timings are reported separately. This debug-profile diagnostic is not a comprehensive session-weighted benchmark and does not justify a general build-speed claim.

Initial original-scan measurement: 68,225 files and four oversized files. Git listing/decode 287.50 ms, ignored-lock discovery/merge 359.88 ms, filtering 57.09 ms, metadata observation 114.51 ms. Same captured rows: insertion median 209.39 ms versus candidate bulk median 57.66 ms. Whole original scans: 853.53, 865.85, 778.14 ms. Concurrent repository edits can change later discovery counts, so only the paired builder comparison holds inputs exactly fixed.

Final paired measurement (`20261029-212244253-2843888`): 68,226 files, four oversized. Insertion median 194.61 ms versus production bulk 58.95 ms. Whole fresh scans 772.26, 751.80, 664.27 ms. The extra file and changing system activity mean these whole-scan observations are not a controlled before/after speed ratio.

Smaller Jcode tree (`20261029-212333422-2850094`): 2,659 files, seven oversized. Insertion median 5.02 ms versus production bulk 2.36 ms on identical observations. Whole fresh scans 23.83, 23.69, 23.30 ms. Seven paired builder samples per tree, unoptimized test profile. These are component measurements, not release-mode or overall workflow claims.
