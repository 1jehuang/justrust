# Recorded runs (frozen archive)

`justrust export` is retired. Every recorded build is now uploaded
automatically to the account-private record store; use
`justrust records list` and `justrust records show run <id>`.

This directory is kept read-only because older docs cite run ids in it. Do
not add files here. One slim JSON line per run, `<machine>/<YYYY-MM>.jsonl`
(plus `-archive-N` siblings); runs from repos not in `public-repos.txt` keep
only numbers.
