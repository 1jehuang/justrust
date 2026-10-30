#!/usr/bin/env python3
"""One-shot historical br48 retention. No builds or compiler invocations.

Not portable replay: use validate.py for that. Only the dedicated private run
homes are exported. Existing artifacts are never overwritten.
"""
import os
from pathlib import Path
import shutil
import subprocess

ROOT = Path(__file__).resolve().parent
SOURCE = Path('/home/jeremy/.jcode/scratch/br48')
LAUNCH = Path('/home/jeremy/.jcode/scratch/ba30')
STACKS = ('owned-off', 'owned-on', 'owned-clif', 'owned-threads4', 'owned-clif-threads4')


def copy(source, destination):
    assert source.is_file() and not source.is_symlink(), source
    destination.parent.mkdir(parents=True, exist_ok=True)
    with source.open('rb') as src, destination.open('xb') as dst:
        shutil.copyfileobj(src, dst)


def main():
    for path in sorted(SOURCE.iterdir()):
        if path.is_file():
            copy(path, ROOT / path.name)
    for name in ('launch-runtime48.sh', 'runtime48.exit', 'runtime48.launch.log',
                 'runtime48.stdout', 'runtime48.stderr'):
        copy(LAUNCH / name, ROOT / name)
    release = SOURCE.parent / 'native-release-frontier-20261030'
    for path in sorted(release.iterdir()):
        if path.is_file():
            copy(path, ROOT / 'release-build' / path.name)
    copy(release / 'acceptance/acceptance.json', ROOT / 'release-build/acceptance/acceptance.json')
    copy(release / 'acceptance/fixture.rs', ROOT / 'release-build/acceptance/fixture.rs')
    for path in sorted((release / 'acceptance/mechanism-profiles').rglob('*.mm_profdata')):
        copy(path, ROOT / 'release-build' / path.relative_to(release))
    repository = Path('/home/jeremy/justrust')
    for name in ('reuse-cgu-inline-accept.py', 'reuse-declarative-macro-accept.py', 'query_profile.py'):
        copy(repository / 'bench' / name, ROOT / 'acceptance-runner' / name)
    for name in ('compiler/rustc_monomorphize/src/partitioning.rs', 'compiler/rustc_session/src/options.rs'):
        result = subprocess.run(['git', '-C', str(repository), 'show',
                                 '66e0f9273:toolchain/rustc/' + name], capture_output=True, check=True)
        path = ROOT / 'release-build/source' / name
        path.parent.mkdir(parents=True, exist_ok=True)
        with path.open('xb') as stream:
            stream.write(result.stdout)
    for stack in STACKS:
        home = SOURCE / stack / 'home/.justrust'
        for path in sorted((home / 'runs').rglob('*')):
            if path.is_file():
                copy(path, ROOT / 'state' / stack / path.relative_to(home))
        runtime = SOURCE / stack / 'runtime/jr-cargo'
        if runtime.is_dir():
            for path in sorted(runtime.iterdir()):
                if path.suffix in ('.out', '.err'):
                    copy(path, ROOT / 'state' / stack / 'runtime/jr-cargo' / path.name)
        command = ['justrust', 'export', '--out', str(ROOT / 'exports'),
                   '--machine', f'xps14-runtime-br48-{stack}-20261030']
        env = dict(os.environ, JUSTRUST_HOME=str(home))
        result = subprocess.run(command, env=env, capture_output=True, check=True)
        (ROOT / f'export-{stack}.stdout').write_bytes(result.stdout)
        (ROOT / f'export-{stack}.stderr').write_bytes(result.stderr)


if __name__ == '__main__':
    main()
