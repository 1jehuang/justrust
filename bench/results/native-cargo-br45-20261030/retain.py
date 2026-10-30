#!/usr/bin/env python3
"""One-shot historical br45 retention. No builds or compiler invocations.

Not portable replay: use validate.py for that. Only the dedicated private run
homes are exported. Existing artifacts are never overwritten.
"""
import os
from pathlib import Path
import shutil
import subprocess

ROOT = Path(__file__).resolve().parent
SOURCE = Path('/home/jeremy/.jcode/scratch/br45')
LAUNCH = Path('/home/jeremy/.jcode/scratch/ba30')
STACKS = ('owned-off', 'owned-on', 'owned-jrcargo', 'owned-jrdaemon', 'owned-jrdaemon4')


def copy(source, destination):
    assert source.is_file() and not source.is_symlink(), source
    destination.parent.mkdir(parents=True, exist_ok=True)
    with source.open('rb') as src, destination.open('xb') as dst:
        shutil.copyfileobj(src, dst)


def main():
    for path in sorted(SOURCE.iterdir()):
        if path.is_file():
            copy(path, ROOT / path.name)
    for name in ('launch-runtime45.sh', 'runtime45.exit', 'runtime45.launch.log',
                 'runtime45.stdout', 'runtime45.stderr'):
        copy(LAUNCH / name, ROOT / name)
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
                   '--machine', f'xps14-runtime-br45-{stack}-20261030']
        env = dict(os.environ, JUSTRUST_HOME=str(home))
        result = subprocess.run(command, env=env, capture_output=True, check=True)
        (ROOT / f'export-{stack}.stdout').write_bytes(result.stdout)
        (ROOT / f'export-{stack}.stderr').write_bytes(result.stderr)


if __name__ == '__main__':
    main()
