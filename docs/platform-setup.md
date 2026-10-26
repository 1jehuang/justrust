# Persistent platform setup

`justrust setup` stores user defaults, never project settings or credentials.
No configuration is created or required for existing build commands.

```sh
# Review Microsoft's SDK terms and confirm you are authorized first:
justrust setup set microsoft-sdk-license 1 --accept-license
justrust setup set install-llvm-tools 1
justrust setup set darwin-sdk <64-lowercase-hex-managed-SDK-ID>
justrust setup set darwin-signing-identity <40-hex-certificate-SHA1>
justrust setup set windows-signing-thumbprint <40-hex-certificate-SHA1>
justrust setup show
justrust setup unset microsoft-sdk-license
```

The Microsoft terms are at <https://go.microsoft.com/fwlink/?LinkId=2086102>.
`--accept-license` is mandatory when setting SDK consent to `1`. LLVM opt-in
is independent and does not accept Microsoft's SDK terms. Set either boolean
to `0` to disable its default, or unset it to return to environment-only behavior.
Setting a Darwin SDK ID does not import, download, or license an SDK. First
provision a lawfully supplied SDK on the build host using
`justrust toolchain import-darwin-sdk --licensed <path>`, then use its ID.

| Key | Accepted value | Default environment variable |
| --- | --- | --- |
| `microsoft-sdk-license` | `0` or `1` | `JUSTRUST_ACCEPT_MICROSOFT_SDK_LICENSE` |
| `install-llvm-tools` | `0` or `1` | `JUSTRUST_INSTALL_LLVM_TOOLS` |
| `darwin-sdk` | exactly 64 lowercase hex characters | `JUSTRUST_DARWIN_SDK` |
| `darwin-signing-identity` | 40 hex characters, certificate SHA1 | none, local signing only |
| `windows-signing-thumbprint` | 40 hex characters, certificate SHA1 | none, local signing only |

Explicit environment variables win on each invocation, including empty strings
and `0`. Defaults are applied before normal build routing and the existing
hosted environment allowlist. For example:

```sh
JUSTRUST_ACCEPT_MICROSOFT_SDK_LICENSE=0 justrust build --target x86_64-pc-windows-msvc
JUSTRUST_DARWIN_SDK=<different-managed-ID> justrust build --target aarch64-apple-darwin
```

`show` prints only persisted values, not merged environment values. Unsetting
never changes your shell environment. Public signing fingerprints are consumed
only by `justrust sign-release` for the matching target, when its explicit
`--identity` or `--thumbprint` argument is absent. They are not exported as
build environment variables or forwarded to hosted builds. No signing is
enabled by storing these settings. Private keys, passwords, P12 paths, PEM data, arbitrary identity
names, and unknown keys are rejected. Hex validation cannot determine whether a
user has mislabeled some other hexadecimal value as a public fingerprint.

Configuration lives in `$JUSTRUST_HOME/platform/config.json`, or
`~/.justrust/platform/config.json` by default. On Unix the dedicated directory is
mode `0700`, new files are `0600`, writes are serialized by a file lock, and a
synced temporary file is atomically renamed over the old configuration. SDK and
signing material is never copied into this file. Persistence currently requires
Unix private-file permissions. Existing environment-only behavior remains
available on other platforms, and Windows users can pass explicit local signing
arguments without a persisted config. Invalid stored configuration fails with a diagnostic
rather than silently accepting consent. Internal compiler/linker wrappers do not
reload user defaults. No global installation is performed by these commands.

## Local signing plans and execution

`justrust sign-release` embeds the same helper as `scripts/sign-release.py` and
requires Python 3 (`python3`, or `python` if that executable is absent). It never
loads keys from config. The OS certificate store/keychain retains private keys.

```sh
# Plan only, using saved Darwin fingerprint:
justrust sign-release --input ./jcode --output ./jcode-signed \
  --sha256 <input-SHA256> --target aarch64-apple-darwin

# Override saved identity explicitly, still plan only:
justrust sign-release --input ./jcode --output ./jcode-signed \
  --sha256 <input-SHA256> --target aarch64-apple-darwin \
  --identity 'Developer ID Application: Example (TEAMID)'

# Windows example. --signtool is always required explicitly, never persisted:
justrust sign-release --input ./jcode.exe --output ./jcode-signed.exe \
  --sha256 <input-SHA256> --target x86_64-pc-windows-msvc \
  --thumbprint <40-hex-SHA1> --signtool 'C:\\SDK\\signtool.exe'
```

Append `--execute` only to sign. Execution requires the matching native OS.
On Windows, use explicit `--thumbprint` since config persistence is Unix-only.
The helper verifies the input SHA256 and architecture, refuses an existing
output, signs a temporary copy, verifies the native signature, and publishes
without overwriting. Darwin verification continues to require an Apple Developer
ID Application chain, including when a certificate fingerprint selects the key.
A fingerprint is not proof that the certificate is authorized. Windows uses the
CurrentUser/My certificate store and Authenticode verification. No private keys,
passwords or imported certificate files are accepted. No notarization is performed.
Saved 64-hex signing references from an earlier config must be replaced with
40-hex SHA1 values before use. `darwin-sdk` remains a 64-hex SHA256 ID.

