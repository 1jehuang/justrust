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
