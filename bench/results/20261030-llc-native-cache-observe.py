#!/usr/bin/env python3
"""Scoped hosted cache experiment with explicit shared-cache worker locality."""
import collections
import hashlib
import json
import os
from pathlib import Path
import platform
import subprocess
import sys
import time

root = Path(__file__).resolve().parent
os.chdir(root)
binary = Path(sys.argv[1]).resolve(strict=True)
policy = sys.argv[2]
assert policy in ('inherit', 'llc')
prefix = root / 'locality-results' / policy
command = ['python3', 'bench/suite.py', 'native-cache', '--binary', str(binary),
           '--scratch', str(root / 'scratch'), '--out-prefix', str(prefix),
           '--stock', '/usr/local/bin/rustc', '-n', '5', '--warmup', '1',
           '--graph-locality', policy]
if Path(str(prefix) + '-off.json').exists():
    command.append('--resume')

def snapshot():
    counts = collections.Counter()
    for path in Path('/proc').glob('[0-9]*/comm'):
        try:
            name = path.read_text().strip()
            if name.startswith(('rustc', 'jr-rust', 'cargo', 'jr-cargo')):
                counts[name] += 1
        except OSError:
            pass
    return {'utc': time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()),
            'loadavg': list(os.getloadavg()),
            'cpu_stat': Path('/proc/stat').read_text().splitlines()[0],
            'compiler_process_counts': dict(counts)}

metadata = {'schema': 1, 'execution': 'authenticated-justrust-hosted-direct-process',
            'ssh_and_transfer_in_timed_samples': False, 'command': command,
            'host': platform.node(), 'machine': platform.machine(),
            'affinity': sorted(os.sched_getaffinity(0)),
            'lscpu': subprocess.check_output(['lscpu'], text=True),
            'cpu_topology': subprocess.check_output(['lscpu', '-e=CPU,CORE,SOCKET,NODE,CACHE'], text=True),
            'stock_version': subprocess.check_output(['/usr/local/bin/rustc', '-vV'], text=True),
            'compiler_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
            'observer_sha256': hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
            'observations': [snapshot()]}
output = Path(str(prefix) + '-observations.json')
output.parent.mkdir(parents=True, exist_ok=True)
def save():
    temporary = output.with_suffix('.json.tmp')
    temporary.write_text(json.dumps(metadata, indent=2) + '\n')
    temporary.replace(output)
save()
process = subprocess.Popen(command)
while True:
    try:
        status = process.wait(timeout=5)
        break
    except subprocess.TimeoutExpired:
        metadata['observations'].append(snapshot())
        save()
metadata['observations'].append(snapshot())
metadata['exit_code'] = status
save()
Path(str(prefix) + '.exit').write_text(str(status) + '\n')
raise SystemExit(status)
