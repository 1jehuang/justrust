#!/usr/bin/env python3
"""Install an explicitly pinned published Linux client into RUNNER_TEMP only.

Example: --tag v0.1.0-alpha.28 --sha256 <reviewed archive digest>
--output-dir must be a new directory below RUNNER_TEMP. With no output directory,
a private unique directory is created there. --github-path appends its directory
to GITHUB_PATH only after checksum, version, and release-marker validation.
No service selection, compilation, default-client replacement, or updater runs.

--resolve-production installs nothing. It prints the release the production
service currently selects, with that release's published archive checksum, as
`bootstrap_tag=` and `bootstrap_sha256=` lines for GITHUB_OUTPUT. Release CI
freezes that pair once and passes it to --tag/--sha256 in every build job, so
the bootstrap follows production instead of a hand-edited pin.
"""
import argparse
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import tarfile
import tempfile
import time
import urllib.parse
import urllib.request

TRIPLE = "x86_64-unknown-linux-musl"
ASSETS = "https://github.com/1jehuang/justrust/releases/download/"
LIMIT = 128 * 1024 * 1024
# The same selection `justrust update` and scripts/production-dogfood.py follow.
PRODUCTION = "https://build.jcode.sh/v1/release/latest"

# Share the production helper's sanitized release-validation environment. The
# production latest-selection and activation paths are deliberately not used.
_spec = importlib.util.spec_from_file_location(
    "production_dogfood", Path(__file__).resolve().parents[1] / "scripts/production-dogfood.py")
production = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(production)


class HTTPSRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        if urllib.parse.urlsplit(newurl).scheme != "https":
            raise ValueError("release redirect must use HTTPS")
        return super().redirect_request(req, fp, code, msg, headers, newurl)


def fetch(url):
    request = urllib.request.Request(url, headers={"User-Agent": "justrust-pinned-bootstrap/1"})
    with urllib.request.build_opener(HTTPSRedirect()).open(request, timeout=60) as response:
        if urllib.parse.urlsplit(response.geturl()).scheme != "https":
            raise ValueError("release download must use HTTPS")
        data = response.read(LIMIT + 1)
    if len(data) > LIMIT:
        raise ValueError("release archive exceeds size limit")
    return data


def pinned_url(tag, sha256):
    if not re.fullmatch(r"v[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z]+(?:\.[0-9A-Za-z]+)*)?", tag):
        raise ValueError("tag must be an exact v-prefixed release version")
    if not re.fullmatch(r"[0-9a-fA-F]{64}", sha256):
        raise ValueError("sha256 must be a fixed trusted 64-digit archive checksum")
    if platform.system() != "Linux" or platform.machine() != "x86_64":
        raise ValueError("published bootstrap requires Linux x86_64")
    return f"{ASSETS}{tag}/justrust-{TRIPLE}.tar.gz"


def production_release(attempts=5, delay=10):
    """The (tag, archive sha256) of the release production selects right now.

    Only the tag is taken from the service. The checksum comes from the fixed
    release asset location for that tag, never from a URL in the response.
    """
    for attempt in range(attempts):
        try:
            info = json.loads(fetch(f"{PRODUCTION}?target={TRIPLE}"))
            tag = info.get("tag") if isinstance(info, dict) else None
            if (not isinstance(tag, str) or not re.fullmatch(r"v[0-9]+\.[0-9]+\.[0-9]+", tag)
                    or info.get("version") != tag[1:] or info.get("enabled") is not True):
                raise ValueError("production service selects no enabled stable release")
            checksum = fetch(f"{ASSETS}{tag}/justrust-{TRIPLE}.tar.gz.sha256")[:4096].decode().split()
            if not checksum or not re.fullmatch(r"[0-9a-fA-F]{64}", checksum[0]):
                raise ValueError("invalid published checksum")
            return tag, checksum[0].lower()
        except (OSError, ValueError):
            if attempt + 1 == attempts:
                raise
            time.sleep(delay)


def executable(archive, sha256):
    if len(archive) > LIMIT or hashlib.sha256(archive).hexdigest() != sha256.lower():
        raise ValueError("release archive checksum mismatch or size limit exceeded")
    root = f"justrust-{TRIPLE}"
    binary = None
    with tarfile.open(fileobj=io.BytesIO(archive), mode="r:gz") as bundle:
        # Stream member headers to avoid unbounded metadata allocation. No member
        # is extracted to disk, and reject links/paths even outside the binary.
        count = 0
        for member in bundle:
            count += 1
            if count > 64:
                raise ValueError("too many release archive members")
            if member.isdir() and member.name.rstrip("/") == root:
                continue
            if (member.name in (f"{root}/README.md", f"{root}/LICENSE")
                    and member.isfile() and 0 <= member.size <= 1024 * 1024):
                continue
            if (member.name != f"{root}/justrust" or not member.isfile()
                    or not 0 < member.size <= LIMIT or not member.mode & 0o111
                    or binary is not None):
                raise ValueError("release archive must contain one regular bounded executable only")
            binary = bundle.extractfile(member).read(LIMIT + 1)
            if len(binary) != member.size:
                raise ValueError("truncated release executable")
    if binary is None:
        raise ValueError("release executable missing")
    return binary


def validate(binary, tag):
    def run(*args):
        return subprocess.run([str(binary), *args], env=production.production_env(),
                              check=True, text=True, stdout=subprocess.PIPE,
                              stderr=subprocess.PIPE, timeout=30).stdout
    if run("--version").strip() != f"justrust {tag[1:]}":
        raise ValueError("release executable version mismatch")
    if "(release build)" not in run("update", "status"):
        raise ValueError("download is not marked as a release build")


def install(tag, sha256, output_dir=None, github_path=False):
    url = pinned_url(tag, sha256)
    runner = os.environ.get("RUNNER_TEMP")
    if not runner:
        raise ValueError("RUNNER_TEMP is required")
    root = Path(runner).resolve(strict=True)
    if not root.is_dir() or any(c in str(root) for c in "\r\n"):
        raise ValueError("invalid RUNNER_TEMP")
    destination = Path(output_dir).absolute() if output_dir else None
    if destination is not None:
        resolved = destination.resolve()
        if (resolved == root or root not in resolved.parents
                or destination.exists() or destination.is_symlink()
                or any(c in str(destination) for c in "\r\n")):
            raise ValueError("output-dir must be a new directory strictly below RUNNER_TEMP")
        destination = resolved
    path_file = os.environ.get("GITHUB_PATH") if github_path else None
    if github_path and not path_file:
        raise ValueError("--github-path requires GITHUB_PATH")
    data = executable(fetch(url), sha256)
    if destination is None:
        destination = Path(tempfile.mkdtemp(prefix="justrust-published-", dir=root))
    else:
        destination.mkdir(mode=0o700)
    try:
        binary = destination / "justrust"
        with binary.open("xb") as out:
            out.write(data)
        binary.chmod(0o700)
        validate(binary, tag)
        if path_file:
            with open(path_file, "a", encoding="utf-8") as out:
                out.write(f"{destination}\n")
        print(f"published bootstrap client: {binary} ({tag}, sha256={sha256.lower()})")
        return binary
    except BaseException:
        shutil.rmtree(destination)
        raise


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--resolve-production", action="store_true")
    parser.add_argument("--tag")
    parser.add_argument("--sha256")
    parser.add_argument("--output-dir")
    parser.add_argument("--github-path", action="store_true")
    args = parser.parse_args()
    if args.resolve_production:
        if args.tag or args.sha256 or args.output_dir or args.github_path:
            parser.error("--resolve-production takes no other option")
        print("bootstrap_tag=%s\nbootstrap_sha256=%s" % production_release())
        return
    if not args.tag or not args.sha256:
        parser.error("--tag and --sha256 are required")
    install(args.tag, args.sha256, args.output_dir, args.github_path)


if __name__ == "__main__":
    main()
