# Verified public toolchain prefixes

`JUSTRUST_TOOLCHAIN_DIST=https://<trusted-public-store>` opts a client into
maintainer-published prefix downloads. This is an executable-code trust root,
not a tenant artifact cache. There is deliberately no automatic public-store
probing when the variable is absent. `justrust toolchain install jr-<commit>`
and an explicit JR project/environment pin can download a candidate. Automatic
installation respects `JUSTRUST_TOOLCHAIN_AUTO_INSTALL=0` on local and hosted
builds. Distribution configuration and automatic-install controls are forwarded
to hosted builds without adding credential forwarding.

Downloaded JR compilers always live in `toolchains/staged/<id>` and receive a
client-created `Installed` marker with `installed_at: 0`. They are available to
explicit pins but cannot become the automatic default, including for old clients
that do not understand a zero timestamp. Downloading does not activate or replace
an existing activated compiler. A normal nightly prefix retains its stock rustc
and may include the compatible shared `bin/jr-cargo` fork.

## Immutable v1 publisher contract

The publisher must make an ID's manifest immutable. The client derives both paths
from the configured trusted HTTPS base, never from a URL inside a manifest:

```text
v1/toolchains/<host-target>/<id>/manifest.json
v1/toolchains/o/<archive-sha256>.tar.gz
```

The JSON manifest accepts exactly these fields:

| Field | Meaning |
| --- | --- |
| `schema_version` | Integer `1` |
| `id`, `target` | Exact requested pinned ID and client host target |
| `rustc_version` | Exact first line of `rustc -vV` |
| `rustc_commit_hash` | Full 40-character lowercase commit hash |
| `cargo_upstream_commit` | Exact 7 to 40-character lowercase commit printed by stock `cargo -V` |
| `jr_cargo` | Optional lowercase fork commit printed in `(jr-cargo <commit>)` |
| `archive_sha256` | Lowercase 64-character SHA-256 of the compressed archive |
| `archive_size` | Exact compressed byte count, at most 8 GiB |
| `unpacked_size` | Exact sum of regular-file payload bytes, at most 32 GiB |
| `components` | Bounded component names, including rustc, cargo, and rust-std |
| `manifest_date` | Original official manifest date, required to match a dated official ID |
| `official_manifest_sha256` | Optional official channel-manifest provenance hash, or null |

The archive contains USTAR entries directly under `bin`, `lib`, `libexec`, `share`, or `etc`.
No `./` root entry, supplied install marker, symlink, hardlink, device, GNU/PAX
extension, duplicate path, absolute path, or parent traversal is accepted. Files
have special permission bits stripped. Extraction is streamed with an expanded
byte budget and at most 500,000 entries. The gzip footer is checked. Archives must
include usable rustc, cargo, rustdoc, and a target standard-library rlib. Declared
optional components receive corresponding executable/source/library checks.
Before publication into the install namespace, rustc's version, full commit, and
host must match. JR commits must also match the requested ID prefix. Stock cargo
and an optional jr-cargo must report the declared upstream commit, and jr-cargo
must report its declared fork commit.

The client writes its own serialized `Installed` marker only after all checks.
A process-wide install lock covers discovery, interrupted-staging cleanup,
verification, marker creation, and the final same-filesystem rename. Existing
complete installations are reused. Failures clean staging, and an incomplete
unmarked destination can be replaced on retry. An official ID first tries a
configured shared prefix, then falls back to the existing verified official dist
installer if the prefix is missing, invalid, or incomplete for the request. JR
prefixes require only supported declared components, not optional official
components such as rustc-dev that a fork may not ship.

## Validation, October 13, 2026 local time

Local-only security validation was explicitly approved while the hosted lease was
reserved for other work. These are correctness observations, not performance
comparisons:

- `20261013-193737970-686755`: `justrust check --local` passed.
- `20261013-194118892-716665`: 16 scoped toolchain unit tests passed, including
  hostile paths/link types, oversized/tampered identities, corrupt/truncated
  archives, incomplete prefixes, binary identity mismatches, concurrent install,
  interrupted staging, idempotence, inactive JR selection, nightly provenance,
  and an offline subprocess test of verified official fallback.
- `20261013-194124908-718563`: all three CLI toolchain integration tests passed,
  including explicit stage versus activation.
- `20261013-194125234-719016`: `justrust clippy --local --all-targets -- -D warnings`
  passed.

Initial fixture runs used a wrong-length synthetic full commit and failed the
intended manifest validation. The fixture now constructs exactly 40 characters.
An existing staged-install fixture also required a valid marker and rustc instead
of an empty JSON object with only cargo. Those failures were fixture issues, not
relaxations of verification. Real published-prefix acceptance is recorded
separately after download and scratch-workload verification.

Final regression run `20261013-194746505-749789` passed all 380 tests, including
independent-process lock serialization and the added `libexec` fixture. Final
all-target warning-denying clippy run `20261013-194811157-758446` passed.

## Real HTTPS acceptance

Both published manifests and their content-addressed archives were downloaded
with the new client into fresh scratch `JUSTRUST_HOME` directories:

| Prefix | Compressed bytes | Payload bytes | Archive SHA-256 | Observed install |
| --- | ---: | ---: | --- | ---: |
| `jr-b7907f1fe8f7` | 457865116 | 1436085865 | `362d1171ba42d98db5941a723af5b658fae807ecc5a43c75e5c1d1758432fcd7` | 41.3s |
| `nightly-2026-10-08` | 618894666 | 1730886274 | `e4e6c65be19fb936a4d8b2fcb5eca1c37521f343be68889961e30a7bc8e8ca64` | 58.6s |

The times describe these downloads and are not a speedup comparison. Both stock
cargo and the portable shared jr-cargo reported upstream `f3865b2a4`, and the
fork reported `125bdc8c1`. Full rustc `-vV` matched each published identity.
Repeated installs preserved marker inode and timestamp. The JR marker stayed
at zero in the staged namespace, and an unpinned fresh-home status selected
system rustc, not JR. A tiny real crate passed check, three tests, and clippy
with each explicit pin, and a feature-gated compile error failed as expected:

| Prefix | Check | Three tests | Clippy | Expected compile failure |
| --- | --- | --- | --- | --- |
| JR | `20261013-194432712-731966` | `20261013-194432826-732012` | `20261013-194432969-732106` | `20261013-194433108-732152` |
| Nightly | `20261013-194913305-764072` | `20261013-194913404-764117` | `20261013-194913596-764214` | `20261013-194913886-764266` |

These eight path-redacted summaries are in `data/runs/xps14-toolchain-download/`.
The first real nightly attempt exposed the standard rust-analyzer `libexec`
directory, which the initial root allowlist omitted. It correctly used official
fallback. Adding the legitimate root and a regression fixture enabled the shared
prefix without relaxing traversal, link, marker, or checksum verification.
No system/default install or existing build host was changed for acceptance.

## Limitations

Transport trust and immutable publication depend on the configured maintainer
store. This is not a manifest-signature scheme and must not be pointed at a tenant
upload endpoint. V1 intentionally rejects links and archive extensions, so a
publisher must produce a dereferenced USTAR prefix. Normal interruption cleanup
and atomic visibility are tested, not power-loss durability. Path/identity tests
are portable where possible, but executable fixture and live workload validation
were performed on Linux x86_64. No build-speed improvement is claimed.
