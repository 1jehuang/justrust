#!/usr/bin/env python3
"""Frozen LLVM rustc disk retention experiment, not hosted/end-to-end performance.

Direct compiler invocation is a local-only native protocol acceptance requirement:
Cargo would skip unchanged compilations. No Cargo build or live RAM tier is used.
Raw commands, bytes, timings, RSS and identities remain in a unique private directory.
The generated receipt is uploaded using the existing account-private bench record API.
"""
import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import statistics
import subprocess
import tempfile
import time

FLAGS = ('-Zjr-cache-associated-items', '-Zjr-cache-small-query-values')
DEFAULT_COMPILER = Path(__file__).resolve().parents[1] / 'toolchain/rustc/build/jr/pgo-use/bin/rustc'


def digest(path):
    h = hashlib.sha256()
    with Path(path).open('rb') as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b''):
            h.update(block)
    return h.hexdigest()


def run(command, rss_file=None):
    command = list(map(str, command))
    start = time.monotonic()
    # wait4 reports this child's own peak RSS, unlike cumulative RUSAGE_CHILDREN.
    with tempfile.TemporaryFile() as stdout, tempfile.TemporaryFile() as stderr:
        p = subprocess.Popen(command, stdout=stdout, stderr=stderr)
        try:
            while True:
                pid, status, usage = os.wait4(p.pid, os.WNOHANG)
                if pid:
                    p.returncode = os.waitstatus_to_exitcode(status)
                    break
                if time.monotonic() - start > 180:
                    p.kill()
                    _, status, usage = os.wait4(p.pid, 0)
                    p.returncode = os.waitstatus_to_exitcode(status)
                    raise TimeoutError('Compiler/program exceeded 180 seconds: ' + repr(command))
                time.sleep(0.002)
        finally:
            if p.returncode is None:
                p.kill()
                p.wait()
        stdout.seek(0)
        stderr.seek(0)
        result = dict(command=command, status=p.returncode,
                      stdout_b64=base64.b64encode(stdout.read()).decode(),
                      stderr_b64=base64.b64encode(stderr.read()).decode(),
                      elapsed_seconds=time.monotonic() - start)
    if rss_file is not None:
        result['peak_rss_kib'] = usage.ru_maxrss
    return result


def observable(record):
    return {key: record[key] for key in ('status', 'stdout_b64', 'stderr_b64')}


def artifact_identity(path):
    """Hash stable file bytes, failing closed if a file changes while read."""
    path = Path(path)
    before = path.stat()
    sha256 = digest(path)
    after = path.stat()
    fields = ('st_dev', 'st_ino', 'st_size', 'st_mtime_ns', 'st_ctime_ns')
    if any(getattr(before, key) != getattr(after, key) for key in fields):
        raise AssertionError('Toolchain artifact changed while hashing: ' + str(path))
    return dict(path=str(path.resolve(strict=True)), bytes=after.st_size, sha256=sha256)


def sysroot_identity(sysroot, target_libdir):
    """Inventory all installed artifact files, not linked compiler source trees.

    Directory symlinks are recorded but not traversed. The installed source links
    point at the entire checkout and are not native inputs for this fixture.
    File symlinks include both their literal link target and resolved byte hash.
    This is artifact identity, not proof of loaded mappings or system C tools.
    """
    sysroot = Path(sysroot).resolve(strict=True)
    target_libdir = Path(target_libdir).resolve(strict=True)
    if not target_libdir.is_relative_to(sysroot):
        raise AssertionError('Target library directory is outside the compiler sysroot')
    files, directory_links = [], []

    def fail_walk(error):
        raise error

    for parent, directories, names in os.walk(sysroot, followlinks=False, onerror=fail_walk):
        for name in sorted(directories):
            path = Path(parent) / name
            if path.is_symlink():
                directory_links.append(dict(relative_path=str(path.relative_to(sysroot)),
                                            target=os.readlink(path)))
                # Only compiler-source symlinks may escape file-byte inventory.
                if path.relative_to(sysroot).as_posix() not in (
                        'lib/rustlib/src/rust', 'lib/rustlib/rustc-src/rust'):
                    raise AssertionError('Unhashed sysroot directory symlink: ' + str(path))
        for name in sorted(names):
            path = Path(parent) / name
            if not path.is_file():
                raise AssertionError('Non-file sysroot artifact: ' + str(path))
            entry = dict(relative_path=str(path.relative_to(sysroot)), **artifact_identity(path))
            if path.is_symlink():
                entry['symlink_target'] = os.readlink(path)
            files.append(entry)
    files.sort(key=lambda entry: entry['relative_path'])
    directory_links.sort(key=lambda entry: entry['relative_path'])
    target_prefix = str(target_libdir.relative_to(sysroot)) + '/'
    target = [entry for entry in files if entry['relative_path'].startswith(target_prefix)]
    for crate in ('core', 'std'):
        if not any(Path(entry['relative_path']).name.startswith('lib' + crate + '-')
                   and entry['relative_path'].endswith('.rlib') for entry in target):
            raise AssertionError('Missing target sysroot ' + crate + ' rlib')
    llvm = [entry for entry in files if Path(entry['relative_path']).name.startswith('libLLVM')]
    if not llvm:
        raise AssertionError('Missing native LLVM library byte identity')
    manifest = dict(files=files, directory_symlinks_not_traversed=directory_links)
    return dict(path=str(sysroot), target_libdir=str(target_libdir),
                manifest_sha256=hashlib.sha256(json.dumps(manifest, sort_keys=True).encode()).hexdigest(),
                llvm_libraries=llvm, **manifest)


def identity(compiler):
    executable = artifact_identity(compiler)
    version = run([compiler, '-vV'])
    sysroot = run([compiler, '--print', 'sysroot'])
    target_libdir = run([compiler, '--print', 'target-libdir'])
    if version['status'] or sysroot['status'] or target_libdir['status']:
        raise AssertionError('Compiler identity query failed')
    sysroot_path = Path(base64.b64decode(sysroot['stdout_b64']).decode().strip())
    target_path = Path(base64.b64decode(target_libdir['stdout_b64']).decode().strip())
    artifacts = sysroot_identity(sysroot_path, target_path)
    drivers = [entry for entry in artifacts['files']
               if Path(entry['relative_path']).name.startswith('librustc_driver')]
    if not drivers:
        raise AssertionError('No driver library found')
    if b'LLVM version:' not in base64.b64decode(version['stdout_b64']):
        raise AssertionError('Compiler does not identify its LLVM version')
    if executable != artifact_identity(compiler):
        raise AssertionError('Compiler executable changed during identity capture')
    return dict(**executable, drivers=drivers,
                identity_scope='installed-sysroot-artifact-bytes-not-loaded-memory-or-system-linker',
                sysroot_artifacts=artifacts, version=observable(version),
                sysroot=observable(sysroot), target_libdir=observable(target_libdir))


def fixture(items):
    parts = ['#![allow(dead_code)]', 'const fn body() -> u64 { 7 }',
             'macro_rules! bias { () => { 2u64 }; }']
    for i in range(items):
        parts.append(f'''mod item_{i} {{
    pub trait Dispatch {{ type Output; const BIAS: u64; fn value(&self) -> Self::Output;
        fn opaque(&self) -> impl Copy {{ 1u8 }} }}
    pub struct Widget;
    impl Dispatch for Widget {{ type Output = u64; const BIAS: u64 = {i};
        fn value(&self) -> u64 {{ Self::BIAS + bias!() }} }}
    pub const fn constant() -> u64 {{ {i} }}
    pub async fn asynchronous() -> u64 {{ constant() }}
    pub fn number() -> u64 {{ let f = || {{ fn nested() -> u64 {{ 1 }} nested() }};
        <Widget as Dispatch>::value(&Widget) + f() + constant() }}
}}''')
    parts.append('fn main() { println!("{}", body() + ' + ' + '.join(f'item_{i}::number()' for i in range(items)) + '); }')
    return '\n'.join(parts) + '\n'


def cases(items):
    source = fixture(items)
    expected = str(7 + items * (items - 1) + 3 * items) + '\n'
    return [('warm', source, expected), ('unchanged-forced-recompile', source, expected),
            ('body-edit', source.replace('body() -> u64 { 7 }', 'body() -> u64 { 8 }'), str(int(expected) + 1) + '\n'),
            ('revert', source, expected),
            ('type-error', source.replace('body() -> u64 { 7 }', 'body() -> u64 { "wrong" }'), None),
            ('recovery', source, expected)]


def command(compiler, root, cache, mode):
    return [compiler, root / 'fixture.rs', '--crate-name=same_rustc_retention',
            '--edition=2024', '-Zcodegen-backend=llvm', '-Copt-level=0', '-Cdebuginfo=0',
            '-Zincremental-verify-ich', *[f'{flag}={"yes" if mode == "on" else "no"}' for flag in FLAGS],
            '-C', f'incremental={cache}', '-o', root / 'fixture']


def cache_size(path):
    files = [p for p in path.rglob('*') if p.is_file()]
    return dict(logical_bytes=sum(p.stat().st_size for p in files),
                allocated_bytes=sum(p.stat().st_blocks * 512 for p in files), files=len(files))


def validate_pair(records, expected):
    off, on = records['off'], records['on']
    if observable(off['compile']) != observable(on['compile']):
        raise AssertionError('Exact compiler status/stdout/diagnostics differ')
    for record in records.values():
        if expected is None:
            if record['compile']['status'] != 1:
                raise AssertionError('Expected ordinary type error')
        else:
            if record['compile']['status'] != 0:
                raise AssertionError('Unexpected compile failure: ' + json.dumps(record))
            wanted = dict(status=0, stdout_b64=base64.b64encode(expected.encode()).decode(), stderr_b64='')
            if observable(record['runtime']) != wanted:
                raise AssertionError('Incorrect runtime output')


def summarize(observations):
    result = {}
    for name, _, _ in cases(1):
        result[name] = {}
        for mode in ('off', 'on'):
            rows = [o['modes'][mode] for o in observations if o['name'] == name]
            result[name][mode] = dict(
                median_seconds=statistics.median(r['compile']['elapsed_seconds'] for r in rows),
                max_peak_rss_kib=max(r['compile']['peak_rss_kib'] for r in rows),
                median_cache_bytes=statistics.median(r['cache']['logical_bytes'] for r in rows))
    return result


def capture(compiler, root, repetitions=4, items=32):
    report = dict(schema=2, scope='local-controlled-frozen-LLVM-disk-retention-not-hosted-not-end-to-end',
                  local_reason='Direct native incremental protocol acceptance must force unchanged recompilation',
                  ram_tier='not-implemented', flags=list(FLAGS), verify_ich=True,
                  repetitions=repetitions, items=items, verdict='incomplete', observations=[])
    try:
        report['identity_before'] = identity(compiler)
        for rep in range(repetitions):
            caches = {mode: root / f'repetition-{rep}' / mode for mode in ('off', 'on')}
            for path in caches.values():
                path.mkdir(parents=True, mode=0o700)
            for index, (name, source, expected) in enumerate(cases(items)):
                observation = dict(repetition=rep, name=name, source=source,
                                   source_sha256=hashlib.sha256(source.encode()).hexdigest(),
                                   expected_stdout=expected, order=['off', 'on'] if (rep + index) % 2 == 0 else ['on', 'off'], modes={})
                report['observations'].append(observation)
                for mode in observation['order']:
                    (root / 'fixture.rs').write_text(source)
                    binary = root / 'fixture'
                    binary.unlink(missing_ok=True)
                    record = {}
                    observation['modes'][mode] = record
                    record['compile'] = run(command(compiler, root, caches[mode], mode), root / 'rss.txt')
                    record['cache'] = cache_size(caches[mode])
                    if record['compile']['status'] == 0:
                        record['artifact_sha256'] = digest(binary)
                        record['runtime'] = run([binary])
                validate_pair(observation['modes'], expected)
        report['summary'] = summarize(report['observations'])
        report['verdict'] = 'passed'
    except BaseException as error:
        report['verdict'] = 'failed'
        report['error'] = str(error)
        raise
    finally:
        try:
            report['identity_after'] = identity(compiler)
            if report.get('identity_before') != report['identity_after']:
                report['verdict'] = 'failed'
                report['identity_error'] = 'Compiler executable/driver/sysroot/LLVM identity changed during experiment'
        except Exception as error:
            report['verdict'] = 'failed'
            report['identity_error'] = str(error)
        (root / (root.name + '.json')).write_text(json.dumps(report, indent=2) + '\n')
    if report['verdict'] != 'passed':
        raise AssertionError(report.get('identity_error', 'Experiment failed'))
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--compiler', type=Path, default=DEFAULT_COMPILER)
    parser.add_argument('--output-parent', type=Path, default=Path(tempfile.gettempdir()))
    parser.add_argument('--repetitions', type=int, default=4)
    parser.add_argument('--items', type=int, default=32)
    args = parser.parse_args()
    if args.repetitions < 2 or not 1 <= args.items <= 128:
        parser.error('Need at least two rotated repetitions and 1..128 items')
    os.umask(0o077)
    root = Path(tempfile.mkdtemp(prefix='same-rustc-retention-', dir=args.output_parent))
    print('Private raw evidence: ' + str(root), flush=True)
    try:
        report = capture(args.compiler.resolve(strict=True), root, args.repetitions, args.items)
        print(json.dumps(dict(verdict=report['verdict'], summary=report['summary']), indent=2))
    finally:
        receipt = root / (root.name + '.json')
        if receipt.exists():
            # Same supported record API as bench/suite.py, no public export.
            p = subprocess.run(['justrust', 'records', 'put', 'bench', root.name, str(receipt)], capture_output=True, text=True)
            print(p.stdout, end='')
            if p.returncode:
                print('Record upload failed, private receipt retained: ' + p.stderr)


if __name__ == '__main__':
    main()
