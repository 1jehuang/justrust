#!/usr/bin/env python3
"""Record Python-only checks. Original-machine private verification is optional."""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import validate as v
from seal import seal


def emit(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True)+'\n')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--verify-private', action='store_true', help='Rehash private originals read-only on original collector machine')
    args = parser.parse_args()
    if args.verify_private:
        receipt = v.load(HERE, 'privacy.json')
        for row in receipt['files']:
            data = Path(row['source']).read_bytes()
            v.require(v.sha(data) == row['original_sha256'] and len(data) == row['original_bytes'], 'private original changed')
        emit(HERE/'checks/private-originals.json', dict(status='passed', originals_rechecked=len(receipt['files']), modified_private_files=0, omitted_environment_entries=sum(e['omitted_entries'] for f in receipt['files'] for e in f['environment']), omitted_environment_names_or_values_published=False))
    seal(HERE)
    proc = subprocess.run([sys.executable, '-B', str(HERE/'test_validate.py')], cwd=HERE, capture_output=True, text=True, timeout=180)
    (HERE/'checks').mkdir(exist_ok=True)
    (HERE/'checks/tests.stdout').write_text(proc.stdout)
    (HERE/'checks/tests.stderr').write_text(proc.stderr)
    emit(HERE/'checks/tests.json', dict(status='passed' if proc.returncode == 0 else 'failed', exit=proc.returncode, command=['python3', '-B', 'test_validate.py'], native_jobs=0))
    v.require(proc.returncode == 0, 'offline mutation suite failed: '+proc.stderr)
    emit(HERE/'checks/validation.json', v.validate(HERE, seal=False))
    seal(HERE)
    with tempfile.TemporaryDirectory(prefix='native-reload-final-relocation-', dir=os.environ.get('JCODE_SCRATCH_DIR')) as temp:
        relocated = Path(temp)/'bundle'
        shutil.copytree(HERE, relocated, ignore=shutil.ignore_patterns('__pycache__'))
        home = Path(temp)/'empty-home'
        home.mkdir()
        env = {'HOME': str(home), 'PATH': '', 'PYTHONDONTWRITEBYTECODE': '1'}
        cmd = [sys.executable, '-I', '-B', str(relocated/'validate.py'), '--audit']
        result = subprocess.run(cmd, cwd=temp, env=env, capture_output=True, text=True, timeout=30)
        v.require(result.returncode == 0, 'relocation failed: '+result.stderr)
        emit(HERE/'checks/relocation.json', dict(status='passed', exit=result.returncode, environment={'HOME': 'fresh empty directory', 'PATH': ''}, cwd='fresh relocated scratch directory', audit='Only bundle reads allowed after stdlib startup. External checkout, directories, writes, native execution and network denied.', result=json.loads(result.stdout)))
    seal(HERE)
    print(json.dumps(v.validate(HERE), indent=2, sort_keys=True))


if __name__ == '__main__':
    main()
