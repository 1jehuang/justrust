#!/usr/bin/env python3
"""Run real-state byte restoration measurements inside the authenticated hosted VM.

No compiler builds. Input must be an immutable state-snapshot from the hosted
compiler experiment. Helpers execute in memory, with temporary files in a sibling
scratch directory. Source cache eviction never targets the original snapshot.
"""
import argparse
import hashlib
import json
from pathlib import Path
import shlex
import subprocess
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('source', help='absolute immutable snapshot path on the hosted VM')
    parser.add_argument('scratch', help='absolute non-overlapping hosted scratch directory')
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    directory = Path(__file__).resolve().parent
    helpers = {name: (directory / (name + '.py')).read_text()
               for name in ('disk_state_restore', 'compressed_state_restore')}
    report = {'schema': 1, 'source': args.source, 'scratch': args.scratch,
              'helper_sha256': {name: hashlib.sha256(code.encode()).hexdigest()
                                for name, code in helpers.items()}, 'measurements': {}}
    for compressed in (False, True):
        for evict in (False, True):
            name = ('zstd' if compressed else 'plain') + ('-guest-cold' if evict else '-warm')
            program = 'import types,sys,json,pathlib\n'
            for module, code in helpers.items():
                program += f'm=types.ModuleType({module!r});sys.modules[{module!r}]=m;exec({code!r},m.__dict__)\n'
            program += f'source=pathlib.Path({args.source!r});scratch=pathlib.Path({args.scratch!r})\n'
            program += "if not scratch.name.startswith('disk-state-'): raise SystemExit('unsafe scratch name')\n"
            program += 'scratch.mkdir(parents=True,exist_ok=True)\n'
            module = 'compressed_state_restore' if compressed else 'disk_state_restore'
            program += f'result=sys.modules[{module!r}].benchmark(source,scratch,workers=(1,4,16),repetitions=3,evict_source_cache={evict!r})\n'
            program += "print('DISK_STATE_JSON='+json.dumps(result))\n"
            started = time.perf_counter()
            result = subprocess.run(['justrust', 'remote', 'ssh', 'python3 -c ' + shlex.quote(program)],
                                    text=True, capture_output=True, timeout=900)
            if result.returncode:
                args.output.with_suffix('.error.log').write_text(result.stdout + result.stderr)
                raise RuntimeError(f'{name} failed, see {args.output.with_suffix(".error.log")}')
            lines = [line.split('=', 1)[1] for line in result.stdout.splitlines()
                     if line.startswith('DISK_STATE_JSON=')]
            if len(lines) != 1:
                raise RuntimeError('expected one remote measurement')
            report['measurements'][name] = json.loads(lines[0])
            args.output.write_text(json.dumps(report, indent=2) + '\n')
            print('JCODE_PROGRESS ' + json.dumps({'measurement': name,
                  'wall': time.perf_counter() - started,
                  'medians': report['measurements'][name]['summaries']}), flush=True)
    print(json.dumps({name: value['summaries'] for name, value in report['measurements'].items()}, indent=2))


if __name__ == '__main__':
    main()
