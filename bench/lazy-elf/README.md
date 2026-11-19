# Lazy ELF prediction candidate (benchmark only)

This isolated crate is not a client feature or a new wire capability. `control`
uses the shipping predictor and `expand:SIZE` always uses its unchanged decoder.
`candidate` builds a private instrumented copy of the encoder from the same
checkout. It changes only which old-file offsets appear in the ordinary recipe.
No parser, mode handling, capability, environment flag, or format selection is
added to the live sender or receiver.

After the hosted disk floor clears, from the repository root run:

```sh
justrust test --manifest-path bench/lazy-elf/Cargo.toml --locked
justrust build --release --manifest-path bench/lazy-elf/Cargo.toml --locked
```

With the benchmark binary on the measurement host, run the commands below
using a new scratch directory for every pair. Include the three original
relinks, identical bytes with different permissions, and a single changed
byte in `.symtab`. Alternate control-first and candidate-first and repeat.

```sh
for mode in control candidate; do
    /usr/bin/time -f 'elapsed=%e user=%U sys=%S maxrss_kib=%M' \
        "$BIN" "$mode" "$OLD" "$NEW" "$WORK/$mode.zst" || exit 1
    wc -c < "$WORK/$mode.zst"
    "$BIN" "expand:$(stat -c %s "$NEW")" "$OLD" "$WORK/$mode.zst" "$WORK/$mode.out" && \
        cmp "$NEW" "$WORK/$mode.out" || exit 1
done
```

GNU time's max RSS is not the simultaneous RSS of encoder and zstd. Collect
peak memory for the whole process tree in a fresh cgroup for each run before
considering promotion. Test actual metadata application in the outer artifact
protocol separately. The candidate caps its additional live index at 4 MiB,
uses at most 128 KiB for section discovery, and does not duplicate section
contents. These limits exclude the shipping codec's basis and tables. Byte
savings measured with an offline replay do not establish native CPU, latency,
peak memory, or end-to-end benefit.
