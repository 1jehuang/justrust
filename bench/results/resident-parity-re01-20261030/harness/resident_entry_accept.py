#!/usr/bin/env python3
"""Operational native entry/namespace gate. Does not compile a project or time builds."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess

ORIGIN = '5d4f78f7577cf7170993b74f735099dd03f5c6ca'


def execute(engine, output):
    output.mkdir(parents=True, mode=0o700, exist_ok=False)
    before = hashlib.sha256(engine.read_bytes()).hexdigest()
    env = {k: v for k, v in os.environ.items()
           if not k.startswith(('JR_ENGINE_', 'RA_JIT_', 'BLAZE_'))}
    report = {'scope': 'native-entry-and-namespace-only-not-language-or-performance',
              'status': 'running', 'sha256_before': before, 'rows': []}

    def require(value, message):
        if not value:
            raise ValueError(message)

    def run(name, arguments, changes=None):
        result = subprocess.run([str(engine)] + arguments, env=dict(env, **(changes or {})),
                                capture_output=True, timeout=30, cwd=output)
        (output / f'{name}.stdout').write_bytes(result.stdout)
        (output / f'{name}.stderr').write_bytes(result.stderr)
        row = {'case': name, 'arguments': arguments, 'status': result.returncode,
               'stdout': result.stdout.decode(errors='replace'),
               'stderr': result.stderr.decode(errors='replace')}
        report['rows'].append(row)
        return row

    try:
        row = run('version-without-home', ['--version'])
        require(row['status'] == 0 and row['stdout'].startswith('jr-engine experimental '), 'native version failed')
        require(ORIGIN in row['stdout'] and re.search(r'build resident-v1-[0-9a-f]{64}\)', row['stdout']),
                'missing exact origin or compiled source identity')
        require(run('no-home', ['status'])['status'] != 0, 'missing home accepted')
        require(run('relative-home', ['status'], {'JR_ENGINE_HOME': 'relative'})['status'] != 0,
                'relative home accepted')
        shared = output / 'shared'
        shared.mkdir(mode=0o755)
        shared.chmod(0o755)
        require(run('shared-home', ['status'], {'JR_ENGINE_HOME': str(shared)})['status'] != 0,
                'shared permissions accepted')
        private = output / 'private'
        private.mkdir(mode=0o700)
        linked = output / 'linked'
        linked.symlink_to(private, target_is_directory=True)
        require(run('linked-home', ['status'], {'JR_ENGINE_HOME': str(linked)})['status'] != 0,
                'linked home accepted')
        redirected = output / 'must-not-exist'
        changes = {'JR_ENGINE_HOME': str(private), 'BLAZE_HOME': str(redirected),
                   'RA_JIT_HOME': str(redirected), 'BLAZE_CACHE': str(redirected),
                   'RA_JIT_CACHE': str(redirected), 'BLAZE_REMOTE': 'invalid-untrusted-remote',
                   'RA_JIT_REMOTE': 'invalid-untrusted-remote'}
        require(run('private-status-hostile-aliases', ['status'], changes)['status'] == 0,
                'positive private status control failed')
        require(not redirected.exists(), 'inherited aliases redirected engine state')
        require((private / 'resident-v1/runtime').is_dir(), 'private runtime not initialized')
        require(any((private / 'resident-v1/store').iterdir()), 'versioned private store absent')
        row = run('reject-management', ['enable'], changes)
        require(row['status'] != 0 and 'does not support' in row['stderr'], 'management command accepted')
        trap_home = output / 'trap-home'
        trap_home.mkdir(mode=0o700)
        (trap_home / 'resident-v1').symlink_to(private / 'resident-v1', target_is_directory=True)
        require(run('linked-namespace', ['status'], {'JR_ENGINE_HOME': str(trap_home)})['status'] != 0,
                'linked namespace accepted')
        for path in (private / 'resident-v1').rglob('*'):
            require(not path.is_symlink(), 'private namespace contains link')
            require(path.stat().st_mode & 0o077 == 0, 'private namespace is group/other accessible')
        report['sha256_after'] = hashlib.sha256(engine.read_bytes()).hexdigest()
        require(report['sha256_after'] == before, 'engine changed during entry acceptance')
        report['status'] = 'passed'
    except BaseException as error:
        report['status'] = 'failed'
        report['error'] = repr(error)
        raise
    finally:
        (output / 'acceptance.json').write_text(json.dumps(report, indent=2) + '\n')
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--engine', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    if not args.engine.is_absolute() or not args.output.is_absolute():
        parser.error('absolute paths required')
    report = execute(args.engine.resolve(), args.output.resolve())
    print(json.dumps({'status': report['status'], 'cases': len(report['rows'])}))


if __name__ == '__main__':
    main()
