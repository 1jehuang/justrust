# Trusted local release signing

`scripts/sign-release.py` is a **manual local handoff**, not a cloud build step.
It never exports, uploads, imports, or reads private key files. It does not accept
passwords, PFX files, keychain passwords, or arbitrary signer flags. Existing Azure
signing in CI remains a separate provider and is not changed by this helper.

## Scope and trust boundary

Build and transfer the unsigned artifact first. Independently obtain its expected
SHA256 from your trusted build record, then run the helper on a trusted signing
host. Do not derive the expected hash from an untrusted download and treat that as
a provenance check. The hash pins bytes, not their origin.

The default is a read-only JSON plan: it hashes and checks the input header but
creates no directories/files, invokes no signer, and accesses no certificate store.
Only `--execute` permits signing. Execution must occur on macOS for Darwin targets
or Windows for Windows targets. CPU emulation is allowed: the host CPU need not
match the artifact CPU. Thin 64-bit Mach-O executables and PE32+ executables for
x86_64 and aarch64 are supported. Universal Mach-O, ELF, DLLs and packages are not.
Header checks reject obvious target mistakes; the native signer/verifier remains
the authoritative binary validator.

The output must not exist and must differ from the source. Its parent directory
must already exist on a trusted local filesystem with hard-link support (for
example APFS or NTFS). The helper makes a private temporary directory next to the
output, copies and rehashes the input, signs only the copy, and requires native
verification to succeed. It atomically publishes by hard-linking the verified
copy to the requested output, then removes the temporary directory. This refuses
a concurrent destination instead of overwriting it. It does not modify the
source or publish an artifact on signing/verification failure. Existing outputs
are preserved. Normal exceptions and timeouts clean up temporary files. As with
other filesystem tools, abrupt process termination or power loss can leave a
temporary directory; inspect and remove it before retrying. No durability claim
is made across power failure. Do not use a directory writable by an adversary.

Native processes receive argument arrays with `shell=False` and a five-minute
per-command timeout. The environment is allowlisted to local home/profile/temp
locations (plus macOS system PATH, or Windows system-root locations). Cloud API
keys, build credentials, arbitrary PATH entries, and other inherited variables
are not forwarded. User-installed signing providers that need additional secret
environment variables are deliberately unsupported. Native output is captured,
not streamed into build logs. Failures yield a nonzero exit and stderr diagnostic.

## macOS Developer ID

Prerequisite: the trusted operator has already provisioned a Developer ID
Application identity in the local keychain, with access authorized by local OS
policy. This helper does not enumerate identities, unlock keychains, or automate
credential prompts. It may fail or time out if local authorization is required.

Example **plan only**, using placeholders rather than a production identity:

```sh
python3 scripts/sign-release.py \
  --input ./unsigned/jcode \
  --output ./signed/jcode \
  --sha256 EXPECTED_64_HEX_SHA256 \
  --target aarch64-apple-darwin \
  --identity 'Developer ID Application: YOUR ORGANIZATION (TEAMID)'
```

After reviewing the hash, target, identity and plan, explicitly add `--execute` on
the authorized macOS host. `--identifier` defaults to the stable `com.jcode.cli`;
keep this stable across releases unless changing product identity intentionally.
The signer uses `/usr/bin/codesign --force --sign IDENTITY --identifier IDENTIFIER
--options runtime --timestamp COPY`. `--force` applies only to the temporary copy
and replaces any existing ad-hoc signature. Verification uses `--verify --strict`
and an explicit Apple Developer ID certificate-chain and identifier requirement.
Ad-hoc (`-`), Apple Development and arbitrary self-signed identities are refused.
Ad-hoc signatures are not trusted distribution signatures.

Developer ID signing and a secure timestamp do **not** constitute notarization,
a Gatekeeper acceptance test, or a guarantee of user trust. This helper does not
submit to notarization, staple tickets, publish releases, or assert those outcomes.
Perform that separately using the project's authorized release procedure.

## Windows installed certificate

Prerequisite: the trusted operator has already installed a code-signing identity
with a usable private key in **CurrentUser/My**, and a trusted Windows SDK
SignTool is installed. Pass its full absolute path. The certificate's public SHA1
thumbprint selects the identity; the file and timestamp digests use SHA256.
No `/f`, `/p`, automatic certificate selection, or raw private-key upload is used.
Machine-store certificates are intentionally outside this helper's scope.

Example PowerShell **plan only**:

```powershell
python scripts/sign-release.py `
  --input .\unsigned\jcode.exe `
  --output .\signed\jcode.exe `
  --sha256 EXPECTED_64_HEX_SHA256 `
  --target x86_64-pc-windows-msvc `
  --thumbprint EXPECTED_40_HEX_CERTIFICATE_THUMBPRINT `
  --signtool 'C:\Program Files (x86)\Windows Kits\10\bin\SDK_VERSION\x64\signtool.exe'
```

After review, add `--execute` on the authorized Windows host. Signing uses
`sign /s My /sha1 THUMBPRINT /fd SHA256 /tr URL /td SHA256 COPY`; verification uses
`verify /pa /all /v COPY`. The default RFC 3161 timestamp endpoint is
`https://timestamp.digicert.com`. `--timestamp-url` permits an explicit HTTPS
endpoint without credentials or query strings. SignTool warning/nonzero exit
codes prevent publication. Native Authenticode verification is not a guarantee
of SmartScreen reputation or successful execution on every end-user machine.

## Tests and evidence

```sh
python3 -m unittest discover -s tests -p sign_release_test.py -v
```

Tests use synthetic binary headers, temporary local files and mocked subprocesses.
They cover non-executing plans, missing/mismatched hashes, wrong OS/architecture,
untrusted identities, argument construction, filtered environment, copy races,
signing and verification failures, timeouts, output collision and cleanup.
They do not use real signing credentials and are **not evidence of a real signed
release**, notarization, or OS trust acceptance. Validate native signing separately
on an explicitly authorized disposable artifact before adopting a release flow.

### Installed CLI checks on October 28, 2026

After reinstalling fix `17c344f2e`, the installed `justrust sign-release` command
produced a non-mutating macOS signing plan for the trusted signed Jcode ARM64
artifact from Actions run `40628778548` (SHA256
`2a60e86c2d2a75888bcc40e3c9ede3663817778afab53dbbad2bc9338068e127`).
Its verifier argument begins with `=anchor apple generic`, selecting inline
requirement syntax rather than a requirement filename. No requested output was
created. The same installed CLI with `--execute` on Linux refused with status 1,
`execution requires the target operating system`, and still created no output.
These checks validate the installed dispatch and wrong-host boundary, not native
macOS signing by this helper. The separate Jcode signing/notarization workflow's
acceptance is recorded in [platform rollout](hosted-platform-rollout.md).

### Native helper acceptance on October 28, 2026

Jcode's protected manual workflow run `40632462506`, attempt 1, passed at source
`001a400fa2969711d48932dc3c40707a749bc54b`, checking out this helper at exact
revision `17c344f2e83f836598b82715950497c0879aa713`. It built a fixed, trusted
C executable on native macOS ARM64 before exposing signing credentials. After
normal required-reviewer approval, an isolated temporary keychain supplied the
dedicated Developer ID identity; credentials were unset before helper execution.

The actual standalone Python helper invoked `/usr/bin/codesign`, passed its native
verifier, and atomically published the signed copy. An additional native verifier
bound the Apple Developer ID chain, `com.jcode.cli`, and exact Team ID, then the
signed fixture executed the expected string. The unsigned input hash was
unchanged. Real wrong-hash execution and existing-destination attempts refused
without producing/changing the requested outputs, and plan-only mode remained
non-mutating. All 17 helper tests and 7 validation fixtures passed in that job.

Public receipt artifact `12791579782` is preserved in the Jcode repository at
`docs/evidence/signing-helper-validation-40632462506.json`. It records unsigned
SHA256 `23d1b729b0f05187fef29dcc0247887e1b31bdf0b2e2e22a9454b652e341d9a5`
and signed SHA256 `095213c0d300ca7dfb5a0acee2d0e81623a2974d941c27d49f6a1e2645a96950`.
Only the public receipt was uploaded. No release, notarization, Windows signing,
or native Rust CLI execution is claimed by this helper-fixture run.

The helper-containing client separately passed hosted check
`20261027-212450633-3366843` and full test run `20261027-212543417-3370608`
(827 passed, with an initial parallel `procfs::tests::reads_child_process` failure
passing its isolated confirmation). Strict clippy initially exposed an unrelated
child-lifecycle fixture timeout cleanup issue, fixed in `7ca2bf765`; its targeted
test `20261027-212650479-3374822` and strict all-targets clippy
`20261027-212656010-3375529` then passed. Cited local run summaries are exported
under `data/runs/`.

## Primary references

Syntax and policy references consulted 2026-10-26:

- Apple, Creating distribution-signed code for macOS:
  <https://developer.apple.com/documentation/xcode/creating-distribution-signed-code-for-the-mac/>
- Apple, TN2206: macOS Code Signing In Depth (verification and requirements):
  <https://developer.apple.com/library/archive/technotes/tn2206/_index.html>
- Apple, TN3161: Inside Code Signing: Certificates (keychain and timestamp behavior):
  <https://developer.apple.com/documentation/technotes/tn3161-inside-code-signing-certificates>
- Microsoft, SignTool (certificate store, thumbprint, digest, timestamp, verification
  and exit-code options):
  <https://learn.microsoft.com/en-us/windows/win32/seccrypto/signtool>
