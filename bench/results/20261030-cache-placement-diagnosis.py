import hashlib
import json
import os
from pathlib import Path
import random
import resource
import subprocess
import tempfile
import time
root = Path('/home/ubuntu/justrust-experiments/cache-speed-20261030-0419')
binary = root / 'bin/jr-rust-locality'
base_env = {'PATH': '/usr/bin:/bin', 'LANG': 'C', 'LC_ALL': 'C'}
assert set(range(8)) <= os.sched_getaffinity(0)
result = {'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(), 'scope': 'targeted placement diagnosis only, not general performance evidence', 'samples': []}
with tempfile.TemporaryDirectory(dir=root / 'scratch', prefix='placement-') as name:
    work = Path(name)
    cache = work / 'cache'
    cache.mkdir()
    cases = [('inherited', 'off'), ('inherited', 'warm'), ('workers', 'off'), ('workers', 'warm'), ('whole_process', 'off'), ('whole_process', 'warm')]
    rng = random.Random(42)
    order = [('inherited', 'fill')]
    for _ in range(9):
        shuffled = cases.copy()
        rng.shuffle(shuffled)
        order += shuffled
    for serial, (placement, state) in enumerate(order):
        output = work / f'out-{serial}'
        env = dict(base_env)
        if placement == 'workers':
            env['JR_GRAPH_LOCALITY'] = 'llc'
        cmd = [str(binary), str(root / 'tests/fixtures/full_programs/18_tokenizer_calculator.rs'), '-o', str(output), '--workers', '8', '--opt', 'final', '--verbose']
        if state != 'off':
            cmd += ['--cache-dir', str(cache)]
        if placement == 'whole_process':
            cmd = ['/usr/bin/taskset', '-c', '0-7'] + cmd
        before = resource.getrusage(resource.RUSAGE_CHILDREN)
        start = time.perf_counter_ns()
        p = subprocess.run(cmd, cwd=work, env=env, capture_output=True, check=True)
        elapsed = time.perf_counter_ns() - start
        after = resource.getrusage(resource.RUSAGE_CHILDREN)
        summary = json.loads(p.stdout)
        if placement == 'workers':
            assert summary['locality_workers'] == 8 and summary['locality_cpus'] == list(range(8))
        if state == 'warm':
            assert b'Native link cache: Hit' in p.stderr
        executed = subprocess.run([str(output)], cwd=work, env=env, capture_output=True, check=True)
        assert executed.stdout == (root / 'tests/fixtures/full_programs/18_tokenizer_calculator.stdout').read_bytes()
        output.unlink()
        row = {'placement': placement, 'state': state, 'wall_ns': elapsed, 'user_s': after.ru_utime-before.ru_utime, 'sys_s': after.ru_stime-before.ru_stime, 'summary': summary, 'stderr': p.stderr.decode(), 'loadavg': list(os.getloadavg())}
        result['samples'].append(row)
        (root / 'locality-results/placement-diagnosis.json').write_text(json.dumps(result, indent=2))
        print(placement, state, round(elapsed/1e6, 2), round(row['user_s'], 2), flush=True)
result['complete'] = True
(root / 'locality-results/placement-diagnosis.json').write_text(json.dumps(result, indent=2))
