"""Scoped native compiler timings, deliberately separate from stable synth scores."""
import hashlib
import json
import os
from pathlib import Path
import platform
import statistics
import subprocess
import tempfile
import time

SCHEMA = 1


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def inventory(roots):
    """Hash every input, not just entry points, including manifests and dependencies."""
    result = {}
    for root in roots:
        root = Path(root).resolve(strict=True)
        paths = sorted(root.rglob('*')) if root.is_dir() else [root]
        for path in paths:
            if any(part in ('.git', 'target', '__pycache__') for part in path.relative_to(root).parts):
                continue
            if path.is_symlink():
                raise ValueError(f'symlink input must be explicitly pinned: {path}')
            if path.is_file():
                result[str(path)] = digest(path)
    return result


def command(binary, source, library, workers, mode, output, args, consumer=False):
    cmd = [str(binary), str(source), '--workers', str(workers), '--edition', args.edition,
           '--opt', args.opt, '--debug-assertions', args.debug_assertions]
    if library:
        cmd.append('--lib')
    for cfg in args.cfg:
        cmd.extend(['--cfg', cfg])
    for external in args.extern + (args.consumer_extern if consumer else []):
        cmd.extend(['--extern', external])
    cmd.extend(['--check'] if mode == 'check' else ['-o', str(output)])
    return cmd


def run(args):
    if args.n < 3 or args.warmup < 1:
        raise ValueError('native-frontend needs at least 3 samples and 1 warmup')
    out = Path(args.out)
    if out.exists():
        raise ValueError(f'refusing to overwrite {out}')
    binary = Path(args.binary).resolve(strict=True)
    source = Path(args.source).resolve(strict=True)
    roots = [source.parent, *args.input_root]
    def normalize(specs):
        normalized = []
        for spec in specs:
            name, sep, path = spec.partition('=')
            if not name or not sep:
                raise ValueError('--extern requires NAME=PATH')
            path = Path(path).resolve(strict=True)
            roots.append(path.parent)
            normalized.append(f'{name}={path}')
        return normalized
    args.extern = normalize(args.extern)
    args.consumer_extern = normalize(args.consumer_extern)
    if args.consumer_extern and not args.consumer:
        raise ValueError('--consumer-extern requires --consumer')
    cases = [('library' if args.lib else 'source', source, args.lib)]
    if args.consumer:
        consumer = Path(args.consumer).resolve(strict=True)
        roots.append(consumer)
        cases.append(('consumer', consumer, False))
    inputs = inventory(roots)
    binary_hash = digest(binary)
    environment = {'PATH': '/usr/bin:/bin', 'LANG': 'C', 'LC_ALL': 'C', 'JR_OPT': args.opt}
    contract = {'inputs': inputs, 'cases': [(name, str(path), lib) for name, path, lib in cases],
                'edition': args.edition, 'opt': args.opt, 'debug_assertions': args.debug_assertions,
                'cfg': args.cfg, 'extern': args.extern, 'consumer_extern': args.consumer_extern,
                'environment': environment, 'cwd': 'fresh-output-directory', 'workers': [1, 8], 'modes': ['check', 'build'],
                'samples': args.n, 'warmups': args.warmup, 'execution': 'local-direct-fresh-process',
                'cache': 'warm-filesystem-fresh-output', 'host': platform.node(),
                'machine': platform.machine(), 'cpu': Path('/proc/cpuinfo').read_text().split('model name')[1].splitlines()[0],
                'cpu_count': os.cpu_count(), 'cpu_affinity': sorted(os.sched_getaffinity(0)),
                'runner_sha256': digest(__file__), 'timeout': args.timeout}
    result = {'native_frontend': SCHEMA, 'contract': contract, 'binary': str(binary),
              'binary_sha256': binary_hash, 'started': time.strftime('%Y-%m-%dT%H:%M:%S%z'),
              'scope': 'Scoped frontend only, contended/uncontrolled. Not comprehensive synthetic evidence.',
              'cells': []}
    with tempfile.TemporaryDirectory(prefix='native-frontend-', dir=args.scratch) as tmp:
        for name, path, library in cases:
            for workers in (1, 8):
                for mode in ('check', 'build'):
                    cell = {'case': name, 'workers': workers, 'mode': mode, 'samples': [], 'warmups': []}
                    for iteration in range(-args.warmup, args.n):
                        if inventory(roots) != inputs or digest(binary) != binary_hash:
                            raise ValueError('input or binary identity changed during measurement')
                        output = Path(tmp) / f'{name}-{workers}-{mode}-{iteration}'
                        cmd = command(binary, path, library, workers, mode, output, args, consumer=name == 'consumer')
                        start = time.perf_counter_ns()
                        proc = subprocess.run(cmd, capture_output=True, timeout=args.timeout,
                                              env=environment, cwd=tmp)
                        wall = time.perf_counter_ns() - start
                        try:
                            summary = json.loads(proc.stdout)
                        except (ValueError, UnicodeDecodeError):
                            summary = None
                        valid = (proc.returncode == 0 and isinstance(summary, dict)
                                 and summary.get('mode') == mode and summary.get('workers') == workers
                                 and summary.get('failures') == 0 and summary.get('capture_complete') is True
                                 and summary.get('events_dropped') == 0
                                 and (mode == 'check' or output.is_file()))
                        sample = {'wall_ns': wall, 'ok': valid, 'returncode': proc.returncode,
                                  'summary': summary, 'stderr': proc.stderr.decode(errors='replace')}
                        if not valid:
                            sample['stdout'] = proc.stdout.decode(errors='replace')
                        cell['samples' if iteration >= 0 else 'warmups'].append(sample)
                        if output.exists():
                            output.unlink()
                    result['cells'].append(cell)
                    print(f'{name} w{workers} {mode}: {statistics.median(s["wall_ns"] for s in cell["samples"])/1e6:.3f} ms '
                          f'({"ok" if all(s["ok"] for s in cell["samples"]) else "BLOCKED"})', flush=True)
    if inventory(roots) != inputs or digest(binary) != binary_hash:
        raise ValueError('input or binary identity changed during measurement')
    out.parent.mkdir(parents=True, exist_ok=True)
    with out.open('x') as handle:
        json.dump(result, handle, indent=2)
        handle.write('\n')
    print(f'saved {out}')


def compare(a, b):
    if a.get('native_frontend') != SCHEMA or b.get('native_frontend') != SCHEMA:
        raise ValueError('compare native frontend results only with native frontend results')
    if not a.get('contract') or a['contract'] != b.get('contract'):
        raise ValueError('incompatible native frontend source/dependency, flags, runner or execution identity')
    expected = [(name, w, mode) for name, _, _ in a['contract']['cases']
                for w in (1, 8) for mode in ('check', 'build')]
    for result in (a, b):
        if [(c['case'], c['workers'], c['mode']) for c in result['cells']] != expected:
            raise ValueError('missing or duplicate native frontend cells')
        for cell in result['cells']:
            if (len(cell['samples']) != a['contract']['samples']
                    or len(cell.get('warmups', [])) != a['contract']['warmups'] or any(
                    s.get('ok') is not True or type(s.get('wall_ns')) is not int or s['wall_ns'] <= 0
                    for s in cell['samples'] + cell.get('warmups', []))):
                raise ValueError('failed or incomplete native frontend samples, no speed comparison')
    lines = ['Scoped native frontend only. Contended/uncontrolled, not comprehensive synthetic improvements.',
             f'A {a["binary_sha256"]}\nB {b["binary_sha256"]}']
    for ca, cb in zip(a['cells'], b['cells']):
        ma, mb = [statistics.median(s['wall_ns'] for s in c['samples']) for c in (ca, cb)]
        lines.append(f'{ca["case"]} w{ca["workers"]} {ca["mode"]}: {ma/1e6:.3f} -> {mb/1e6:.3f} ms ({100*(mb/ma-1):+.1f}%), uncontrolled')
    return '\n'.join(lines)


def add_parser(sub):
    p = sub.add_parser('native-frontend', description=__doc__)
    p.add_argument('--binary', required=True)
    p.add_argument('--source', required=True)
    p.add_argument('--lib', action='store_true')
    p.add_argument('--consumer')
    p.add_argument('--consumer-extern', action='append', default=[], help='NAME=PATH source dependency for consumer only')
    p.add_argument('--extern', action='append', default=[])
    p.add_argument('--input-root', action='append', default=[], help='pin full project/dependency tree, repeatable')
    p.add_argument('--edition', default='2024')
    p.add_argument('--opt', choices=('off', 'adaptive', 'final'), default='off')
    p.add_argument('--debug-assertions', choices=('yes', 'no'), default='yes')
    p.add_argument('--cfg', action='append', default=[])
    p.add_argument('-n', type=int, default=5)
    p.add_argument('--warmup', type=int, default=1)
    p.add_argument('--timeout', type=float, default=120)
    p.add_argument('--scratch', required=True)
    p.add_argument('-o', '--out', required=True)
    p.set_defaults(fn=run)
