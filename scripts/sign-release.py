#!/usr/bin/env python3
"""Explicit, hash-pinned local signing. Never reads signing key material."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import struct
import subprocess
import sys
import tempfile


TARGETS = {
    'aarch64-apple-darwin': ('Darwin', 'mach', 0x0100000C),
    'x86_64-apple-darwin': ('Darwin', 'mach', 0x01000007),
    'x86_64-pc-windows-msvc': ('Windows', 'pe', 0x8664),
    'aarch64-pc-windows-msvc': ('Windows', 'pe', 0xAA64),
}


def validate_header(path, target):
    _, kind, cpu = TARGETS[target]
    with path.open('rb') as f:
        head = f.read(64)
        if kind == 'mach':
            if len(head) < 32 or head[:4] != b'\xcf\xfa\xed\xfe':
                raise ValueError('expected a thin little-endian 64-bit Mach-O executable')
            actual, _, filetype = struct.unpack_from('<III', head, 4)
            if actual != cpu or filetype != 2:
                raise ValueError('Mach-O architecture or executable type mismatch')
        else:
            if len(head) < 64 or head[:2] != b'MZ':
                raise ValueError('expected a PE executable')
            offset = struct.unpack_from('<I', head, 60)[0]
            if offset < 64:
                raise ValueError('invalid PE offset')
            f.seek(offset)
            pe = f.read(26)
            if (len(pe) != 26 or pe[:4] != b'PE\0\0'
                    or struct.unpack_from('<H', pe, 4)[0] != cpu
                    or not struct.unpack_from('<H', pe, 22)[0] & 2
                    or struct.unpack_from('<H', pe, 22)[0] & 0x2000
                    or struct.unpack_from('<H', pe, 24)[0] != 0x20B):
                raise ValueError('PE architecture or executable header mismatch')


def digest(path):
    h = hashlib.sha256()
    with path.open('rb') as f:
        for block in iter(lambda: f.read(1024 * 1024), b''):
            h.update(block)
    return h.hexdigest()


def parser():
    p = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    p.add_argument('--input', required=True, type=Path)
    p.add_argument('--output', required=True, type=Path)
    p.add_argument('--sha256', required=True)
    p.add_argument('--target', required=True, choices=TARGETS)
    p.add_argument('--identity', help='full Developer ID Application name or 40-hex keychain certificate SHA1')
    p.add_argument('--identifier', default='com.jcode.cli')
    p.add_argument('--thumbprint', help='installed Windows CurrentUser/My certificate SHA1')
    p.add_argument('--signtool', type=Path, help='absolute trusted Windows SDK signtool.exe path')
    p.add_argument('--timestamp-url', default='https://timestamp.digicert.com')
    p.add_argument('--execute', action='store_true')
    return p


def commands(a, artifact):
    if TARGETS[a.target][0] == 'Darwin':
        if not a.identity or not (a.identity.startswith('Developer ID Application: ')
                                  or re.fullmatch(r'[A-Fa-f0-9]{40}', a.identity)):
            raise ValueError('a Developer ID Application name or 40-hex certificate SHA1 is required; ad-hoc is not trusted')
        if a.thumbprint or a.signtool:
            raise ValueError('Windows signing arguments cannot be used for macOS')
        if not re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9.-]*', a.identifier):
            raise ValueError('invalid signing identifier')
        # codesign treats -R arguments as filenames unless prefixed with '='.
        requirement = ('=anchor apple generic and certificate 1[field.1.2.840.113635.100.6.2.6] exists '
                       'and certificate leaf[field.1.2.840.113635.100.6.1.13] exists '
                       f'and identifier "{a.identifier}"')
        return [
            ['/usr/bin/codesign', '--force', '--sign', a.identity, '--identifier', a.identifier,
             '--options', 'runtime', '--timestamp', str(artifact)],
            ['/usr/bin/codesign', '--verify', '--strict', '--verbose=2', '-R', requirement, str(artifact)],
        ]
    if a.identity:
        raise ValueError('macOS identity cannot be used for Windows')
    if not a.thumbprint or not re.fullmatch(r'[A-Fa-f0-9]{40}', a.thumbprint):
        raise ValueError('a 40-hex installed certificate thumbprint is required')
    if not a.signtool or not a.signtool.is_absolute():
        raise ValueError('an absolute trusted signtool path is required')
    if not re.fullmatch(r'https://[A-Za-z0-9.-]+(?::[0-9]+)?(?:/[A-Za-z0-9./_-]*)?', a.timestamp_url):
        raise ValueError('timestamp URL must be HTTPS without credentials or query')
    return [
        [str(a.signtool), 'sign', '/s', 'My', '/sha1', a.thumbprint, '/fd', 'SHA256',
         '/tr', a.timestamp_url, '/td', 'SHA256', str(artifact)],
        [str(a.signtool), 'verify', '/pa', '/all', '/v', str(artifact)],
    ]


def minimum_env(system):
    keys = ('HOME', 'USER', 'LOGNAME', 'TMPDIR') if system == 'Darwin' else (
        'SystemRoot', 'WINDIR', 'USERPROFILE', 'APPDATA', 'LOCALAPPDATA', 'TEMP', 'TMP')
    env = {key: os.environ[key] for key in keys if key in os.environ}
    if system == 'Darwin':
        env['PATH'] = '/usr/bin:/bin'
    return env


def run(a):
    if not re.fullmatch(r'[A-Fa-f0-9]{64}', a.sha256):
        raise ValueError('expected SHA256 must be 64 hex characters')
    source = a.input.resolve(strict=True)
    output = a.output.absolute()
    if not source.is_file() or source == output.resolve() or os.path.lexists(output):
        raise ValueError('output must be distinct and not already exist; input must be a file')
    if not output.parent.is_dir():
        raise ValueError('output parent must already exist')
    if digest(source) != a.sha256.lower():
        raise ValueError('input SHA256 mismatch')
    validate_header(source, a.target)
    planned = commands(a, '<temporary-artifact>')
    system = TARGETS[a.target][0]
    if not a.execute:
        print(json.dumps({'mode': 'plan-only', 'target': a.target, 'output': str(output),
                          'commands': planned, 'requires_host': system}, indent=2))
        return
    if platform.system() != system:
        raise ValueError('execution requires the target operating system')
    # Copy first, then pin the actual bytes that will be signed, closing input races.
    published = None
    try:
        with tempfile.TemporaryDirectory(prefix='.sign-release-', dir=output.parent) as directory:
            artifact = Path(directory) / ('artifact.exe' if system == 'Windows' else 'artifact')
            shutil.copyfile(source, artifact)
            shutil.copymode(source, artifact)
            if digest(artifact) != a.sha256.lower():
                raise ValueError('copied input SHA256 mismatch')
            validate_header(artifact, a.target)
            for command in commands(a, artifact):
                subprocess.run(command, check=True, shell=False, env=minimum_env(system),
                               stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                               stderr=subprocess.PIPE, timeout=300)
            # Same-filesystem hard link is atomic and refuses an existing destination.
            # Record identity before linking so cleanup failures can undo only our output.
            stat = artifact.stat()
            identity = (stat.st_dev, stat.st_ino)
            os.link(artifact, output)
            published = identity
    except BaseException:
        if published is not None:
            try:
                stat = output.lstat()
                if (stat.st_dev, stat.st_ino) == published:
                    output.unlink()
            except FileNotFoundError:
                pass
        raise
    print(json.dumps({'mode': 'signed-and-native-verified', 'output': str(output)}))


def main():
    try:
        run(parser().parse_args())
    except (ValueError, OSError, subprocess.SubprocessError) as error:
        print(f'signing refused or failed: {error}', file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    sys.exit(main())
