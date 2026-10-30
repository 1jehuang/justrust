#!/usr/bin/env python3
"""Native scalar retention flag transitions. Correctness only, never timing evidence."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys

FLAG = '-Zjr-cache-small-query-values'
FROZEN = '/home/jeremy/.jcode/scratch/query-retention-bench-20261030/home/toolchains/staged/jr-50f30aea8dbb/bin/rustc'


def digest(path):
    h = hashlib.sha256()
    with Path(path).open('rb') as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b''):
            h.update(chunk)
    return h.hexdigest()


def cases():
    path = Path(__file__).with_name('reuse-query-accept.py')
    spec = importlib.util.spec_from_file_location('scalar_accept_cases', path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module.cases()


def run(command, evidence):
    """Persist exact byte streams even on timeout or process launch failure."""
    command = [str(x) for x in command]
    record = dict(command=command, status=None)
    try:
        result = subprocess.run(command, capture_output=True, timeout=180)
        record['status'] = result.returncode
        stdout, stderr = result.stdout, result.stderr
    except subprocess.TimeoutExpired as error:
        record['error'] = 'timeout'
        stdout, stderr = error.stdout or b'', error.stderr or b''
    except OSError as error:
        record['error'] = repr(error)
        stdout, stderr = b'', b''
    evidence.with_suffix('.stdout').write_bytes(stdout)
    evidence.with_suffix('.stderr').write_bytes(stderr)
    record.update(stdout_hex=stdout.hex(), stderr_hex=stderr.hex())
    evidence.with_suffix('.json').write_text(json.dumps(record, indent=2) + '\n')
    return record


def exact(record):
    return tuple(record[k] for k in ('status', 'stdout_hex', 'stderr_hex'))


def require(condition, message):
    if not condition:
        raise AssertionError(message)


def identity(rustc, root):
    version = run([rustc, '-vV'], root / 'version')
    sysroot = run([rustc, '--print', 'sysroot'], root / 'sysroot')
    require(version['status'] == sysroot['status'] == 0, 'Compiler identity query failed')
    prefix = Path(bytes.fromhex(sysroot['stdout_hex']).decode().strip())
    drivers = sorted(p.resolve() for p in (prefix / 'lib').glob('*rustc_driver*') if p.is_file())
    require(bool(drivers), 'Missing native rustc_driver')
    return dict(path=str(rustc), sha256=digest(rustc), version=version,
                sysroot=sysroot, drivers=[dict(path=str(p), sha256=digest(p)) for p in drivers])


def capture(rustc, root):
    root = Path(root).absolute()
    root.mkdir(mode=0o700, exist_ok=False)
    os.chmod(root, 0o700)
    report = dict(schema=1, verdict='incomplete', scope='scalar-flag-transition-correctness-only',
                  verify_ich=True, sequence=['no', 'yes', 'no'], observations=[], completed=0)
    try:
        rustc = Path(rustc).resolve(strict=True)
        report['compiler'] = identity(rustc, root)
        report['harnesses'] = [dict(path=str(p.resolve()), sha256=digest(p)) for p in
                               (Path(__file__), Path(__file__).with_name('reuse-query-accept.py'))]
        work = root / 'work'
        work.mkdir(mode=0o700)
        shared = root / 'shared-cache'
        shared.mkdir(mode=0o700)
        env_keys = ('RUSTC_BOOTSTRAP', 'RUSTFLAGS', 'RUSTC_WRAPPER', 'LD_LIBRARY_PATH')
        report['environment'] = {k: os.environ.get(k) for k in env_keys}
        matrix = cases()
        require(bool(matrix), 'Empty acceptance matrix')
        report['planned'] = len(matrix) * len(report['sequence'])
        for index, (name, source, dependency, expected) in enumerate(matrix):
            step = root / f'{index:02d}-{name}'
            step.mkdir(mode=0o700)
            (work / 'main.rs').write_text(source)
            (work / 'dep.rs').write_text(dependency)
            (step / 'main.rs').write_text(source)
            (step / 'dep.rs').write_text(dependency)
            observation = dict(name=name, expected_stdout=expected, transitions=[])
            report['observations'].append(observation)
            for phase, value in enumerate(report['sequence']):
                attempt = step / f'{phase}-{value}'
                attempt.mkdir(mode=0o700)
                fresh = attempt / 'fresh-cache'
                fresh.mkdir(mode=0o700)
                transition = dict(value=value, modes={})
                observation['transitions'].append(transition)
                # Same source, output and extern paths: diagnostics need NO normalization.
                for mode, cache in (('fresh', fresh), ('shared', shared)):
                    record = {}
                    transition['modes'][mode] = record
                    flags = ['--edition=2024', '-Zincremental-verify-ich',
                             f'{FLAG}={value}', '-Cdebuginfo=0']
                    for crate, source_file, output, extra in (
                        ('retention_dep', 'dep.rs', 'libretention_dep.rlib', ['--crate-type=rlib']),
                        ('retention_fixture', 'main.rs', 'fixture',
                         ['--extern', f'retention_dep={work / "libretention_dep.rlib"}'])):
                        artifact = work / output
                        artifact.unlink(missing_ok=True)
                        command = [rustc, work / source_file, f'--crate-name={crate}',
                                   '-C', f'incremental={cache / crate}', '-o', artifact] + extra + flags
                        record[crate] = run(command, attempt / f'{mode}-{crate}')
                        status = record[crate]['status']
                        wanted = 1 if crate == 'retention_fixture' and expected is None else 0
                        require(status == wanted, f'{name}/{value}/{mode}/{crate}: expected status {wanted}, got {status}')
                    if expected is not None:
                        record['artifact_sha256'] = digest(work / 'fixture')
                        record['runtime'] = run([work / 'fixture'], attempt / f'{mode}-runtime')
                        require(exact(record['runtime']) == (0, expected.encode().hex(), ''),
                                f'{name}/{value}/{mode}: incorrect runtime')
                for kind in ('retention_dep', 'retention_fixture') + (() if expected is None else ('runtime',)):
                    require(exact(transition['modes']['fresh'][kind]) == exact(transition['modes']['shared'][kind]),
                            f'{name}/{value}/{kind}: exact fresh/shared mismatch')
                report['completed'] += 1
        # Rehash the frozen compiler and all identified drivers after the entire matrix.
        for item in [report['compiler']] + report['compiler']['drivers']:
            require(digest(item['path']) == item['sha256'], 'Compiler/driver changed during acceptance')
        report['verdict'] = 'passed'
    except BaseException as error:
        report['verdict'] = 'failed'
        report['error'] = repr(error)
        raise
    finally:
        (root / 'acceptance.json').write_text(json.dumps(report, indent=2) + '\n')
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--rustc', type=Path, default=Path(FROZEN))
    parser.add_argument('--output', type=Path, required=True, help='New private directory, existing parent')
    args = parser.parse_args()
    os.umask(0o077)
    result = capture(args.rustc, args.output)
    print(json.dumps(dict(verdict=result['verdict'], completed=result['completed'], output=str(args.output))))


if __name__ == '__main__':
    try:
        main()
    except Exception as error:
        print(repr(error), file=sys.stderr)
        sys.exit(1)
