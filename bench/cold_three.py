#!/usr/bin/env python3
"""First build on a fresh checkout: stock Cargo on this machine versus
justrust hosted, as a brand-new account would see it.

Three pinned public release builds, each producing the shipping binary:

  jcode    build --release -p jcode --bin jcode --locked   -> target/release/jcode
  ripgrep  build --release --locked                        -> target/release/rg
  uv       build --release -p uv --locked                  -> target/release/uv

Every sample is a new `git archive` extraction at a new path with an empty
target directory, so neither stack has build state for it.

  stock   /usr/bin/cargo with RUSTC=/usr/bin/rustc. The crate sources are
          fetched once beforehand and not measured (`cargo fetch --locked`),
          which favors stock: only compiling and linking are timed.
  hosted  `justrust build --remote` with a unique JUSTRUST_DEPCACHE_NAMESPACE,
          so the hosted machine's own dependency and workspace caches start
          empty, exactly like a new account's. The public prebuilt store
          (dependencies compiled by the prebuild fleet) still applies: that
          is the product's first-build path. The checkout path is new, so the
          hosted target directory is new too.

Wall time runs from process start to exit, and the sample only counts when
the release binary is on THIS machine afterwards and runs `--version`.
Hosted samples must also have run on a hosted machine (summary
remote.machine) and report how many dependency units came from the public
prebuilt store. Stack order alternates per round.

  bench/cold_three.py [--rounds 2] [--only uv,ripgrep] [--stacks stock,remote]
                      [--result bench/results/cold-three-....json]

`--result` appends to an existing result (for example the hosted half of a
run whose stock half is already measured).
"""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import platform
import shutil
import statistics
import subprocess as sp
import sys
import tempfile
import time
import uuid

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import readme_uv as uv  # noqa: E402

ROOT = uv.scratch_root() / 'cold-three'
STOCK_CARGO = '/usr/bin/cargo'

# name: source repo, git url, pinned commit (a release tag where there is
# one), cargo args after `build`, produced binary
ROWS = {
    'jcode': (Path.home() / 'jcode', 'https://github.com/1jehuang/jcode.git',
              'b33d94f006ce2488bbc3b4b5ec818505ac750b28',
              ['--release', '-p', 'jcode', '--bin', 'jcode', '--locked'], 'jcode'),
    'ripgrep': (ROOT / 'src-ripgrep', 'https://github.com/BurntSushi/ripgrep.git',
                'e89fff89ac9af12e8d4ce9d5fd07beb408ca730f',  # 15.2.0
                ['--release', '--locked'], 'rg'),
    'uv': (ROOT / 'src-uv', 'https://github.com/astral-sh/uv.git',
           '6b2d39dcc390293d51c9c0d2bbcd650f4bcbfdce',  # 0.13.9
           ['--release', '-p', 'uv', '--locked'], 'uv'),
}


def source_for(name: str) -> Path:
    source, url, pin = ROWS[name][:3]
    if not source.exists():
        sp.run(['git', 'init', '-q', str(source)], check=True)
    have = sp.run(['git', '-C', str(source), 'cat-file', '-t', pin], capture_output=True, text=True)
    if have.stdout.strip() != 'commit':
        sp.run(['git', '-C', str(source), 'fetch', '-q', '--depth', '1', url, pin], check=True)
    return source


def archive_for(name: str) -> Path:
    archive = ROOT / f'{name}-{ROWS[name][2][:12]}.tar'
    if not archive.exists():
        tmp = archive.with_suffix('.tmp')
        sp.run(['git', '-C', str(source_for(name)), 'archive', '--format=tar', f'--output={tmp}', ROWS[name][2]], check=True)
        tmp.rename(archive)
    return archive


def checkout(name: str, archive: Path, tag: str, stack: str) -> Path:
    return uv.extract_checkout(archive, ROOT / 'runs' / f'{name}-{tag}-{stack}-{uuid.uuid4().hex[:8]}')


def prefetch(name: str, archive: Path) -> None:
    """Crate sources for stock, once, unmeasured."""
    probe = ROOT / 'runs' / f'{name}-fetch-{uuid.uuid4().hex[:8]}'
    uv.extract_checkout(archive, probe)
    env = uv.sample_environment('stock', probe / 'target', None)
    sp.run([STOCK_CARGO, 'fetch', '--locked'], cwd=probe, env=env, check=True,
           stdout=sp.DEVNULL, stderr=sp.DEVNULL)
    shutil.rmtree(probe)


def binary_ok(path: Path) -> tuple[bool, str]:
    if not path.is_file() or not os.access(path, os.X_OK):
        return False, f'missing {path.name}'
    try:
        out = sp.run([str(path), '--version'], capture_output=True, text=True, timeout=60)
    except (OSError, sp.TimeoutExpired) as error:
        return False, f'{path.name} --version: {error}'
    first = (out.stdout or out.stderr).strip().splitlines()[:1]
    return out.returncode == 0, (first[0] if first else f'exit {out.returncode}')


def power_state() -> dict:
    """This laptop's power source and CPU policy: battery mode slows stock."""
    def read(path):
        try:
            return Path(path).read_text().strip()
        except OSError:
            return None
    supplies = Path('/sys/class/power_supply')
    online = [read(p / 'online') for p in supplies.glob('*') if (p / 'online').exists()] if supplies.exists() else []
    return {'ac': '1' in online, 'battery': read(next(iter(sorted(supplies.glob('BAT*/status'))), '/nonexistent')) if supplies.exists() else None,
            'epp': read('/sys/devices/system/cpu/cpu0/cpufreq/energy_performance_preference'),
            'governor': read('/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor'),
            'platform_profile': read('/sys/firmware/acpi/platform_profile')}


def run_one(name: str, stack: str, archive: Path, out: Path, tag: str) -> dict:
    args, binary = ROWS[name][3], ROWS[name][4]
    work = checkout(name, archive, tag, stack)
    rid = 'cold-three-' + uuid.uuid4().hex
    env = uv.sample_environment(stack, work / 'target', rid)
    namespace = None
    if stack == 'stock':
        cmd = [STOCK_CARGO, 'build', *args]
    else:
        namespace = 'cold-' + uuid.uuid4().hex[:20]
        env['JUSTRUST_DEPCACHE_NAMESPACE'] = namespace
        cmd = ['justrust', 'build', '--remote', *args]
    log = out / f'{name}-{tag}-{stack}.log'
    power = power_state()
    start = time.monotonic()
    with log.open('w') as stream:
        rc = sp.run(cmd, cwd=work, env=env, stdout=stream, stderr=sp.STDOUT).returncode
    wall = time.monotonic() - start
    ok, version = binary_ok(work / 'target' / 'release' / binary)
    sample = {'repo': name, 'stack': stack, 'tag': tag, 'wall': wall, 'exit': rc, 'log': str(log),
              'checkout': str(work), 'run_id': rid if stack != 'stock' else None,
              'namespace': namespace, 'binary': version, 'power': power, 'errors': []}
    if rc:
        sample['errors'].append(f'exit {rc}')
    if not ok:
        sample['errors'].append(f'binary not usable here: {version}')
    if stack != 'stock':
        try:
            summary, meta = uv.read_evidence(Path(os.environ.get('JUSTRUST_HOME', Path.home() / '.justrust')), rid)
        except (OSError, ValueError, KeyError) as error:
            sample['errors'].append(f'no run record: {error}')
            return sample
        units = summary.get('units') or {}
        remote = summary.get('remote') or {}
        if not remote.get('machine'):
            sample['errors'].append('fell back locally: no remote.machine')
        if summary.get('exit') != 0:
            sample['errors'].append(f"summary exit {summary.get('exit')}")
        sample.update({
            'machine': remote.get('machine'), 'toolchain': summary.get('toolchain'),
            'real_rustc': meta.get('real_rustc'),
            'units_compiled': units.get('compiled'),
            'depcache_hits': units.get('depcache_hits'), 'depcache_misses': units.get('depcache_misses'),
            'prebuilt_hits': units.get('depcache_prebuilt_hits'),
            'localcache_hits': units.get('localcache_hits'),
            'remote_phases': {'sync_ms': remote.get('sync_ms'), **(remote.get('run') or {})},
            'host_wall': summary.get('wall'),
        })
    return sample


def summarize(samples: list[dict]) -> dict:
    out = {}
    for name in dict.fromkeys(s['repo'] for s in samples):
        row = {}
        for stack in ('stock', 'remote'):
            mine = [s for s in samples if s['repo'] == name and s['stack'] == stack]
            good = [s for s in mine if not s['errors']]
            walls = [s['wall'] for s in good]
            row[stack] = {'n': len(good), 'failed': len(mine) - len(good),
                          'median': statistics.median(walls) if walls else None,
                          'min': min(walls) if walls else None, 'max': max(walls) if walls else None}
            if stack == 'remote' and good:
                row[stack]['prebuilt_hits'] = [s.get('prebuilt_hits') for s in good]
                row[stack]['depcache_misses'] = [s.get('depcache_misses') for s in good]
        out[name] = row
    return out


def identity() -> dict:
    def text(cmd):
        try:
            return sp.run(cmd, capture_output=True, text=True, timeout=30).stdout.strip()
        except OSError:
            return None
    jr = shutil.which('justrust')
    return {'stock_cargo': text([STOCK_CARGO, '--version']), 'stock_rustc': text(['/usr/bin/rustc', '--version']),
            'justrust': text(['justrust', '--version']),
            'justrust_sha256': uv.sha256(os.path.realpath(jr)) if jr else None,
            'machine': platform.node(), 'cpu_count': os.cpu_count(),
            'cpu': next((l.split(':', 1)[1].strip() for l in open('/proc/cpuinfo') if l.startswith('model name')), None)}


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--rounds', type=int, default=2)
    parser.add_argument('--only', default=','.join(ROWS))
    parser.add_argument('--stacks', default='stock,remote')
    parser.add_argument('--result', type=Path, help='append to this result file')
    parser.add_argument('--keep', action='store_true', help='keep checkouts (default: delete after each sample)')
    args = parser.parse_args(argv)
    names = [n for n in args.only.split(',') if n]
    stacks = tuple(s for s in args.stacks.split(',') if s)
    if set(names) - set(ROWS) or set(stacks) - {'stock', 'remote'}:
        parser.error('unknown repo or stack')
    ROOT.mkdir(parents=True, exist_ok=True)
    out = Path(tempfile.mkdtemp(dir=ROOT, prefix='logs-'))
    if args.result and args.result.exists():
        result_path = args.result
        result = json.loads(result_path.read_text())
        result.setdefault('identities', []).append(identity())
    else:
        rev = sp.run(['git', '-C', str(HERE.parent), 'rev-parse', '--short=12', 'HEAD'], capture_output=True, text=True).stdout.strip()
        result_path = args.result or HERE / 'results' / f"cold-three-{time.strftime('%Y%m%d-%H%M%S')}-{rev}.json"
        result = {'schema': 1, 'what': 'first build of a fresh checkout, stock cargo on this machine vs justrust hosted as a new account',
                  'started': time.strftime('%Y%m%d-%H%M%S'), 'identities': [identity()],
                  'rows': {n: {'pin': ROWS[n][2], 'url': ROWS[n][1], 'command': 'build ' + ' '.join(ROWS[n][3]),
                               'binary': ROWS[n][4]} for n in ROWS},
                  'samples': []}
    print(f'logs {out}\nresult {result_path}', flush=True)

    def save():
        result['summary'] = summarize(result['samples'])
        result_path.parent.mkdir(exist_ok=True)
        result_path.write_text(json.dumps(result, indent=1) + '\n')

    for name in names:
        try:
            archive = archive_for(name)
            if 'stock' in stacks:
                prefetch(name, archive)
        except (OSError, sp.CalledProcessError, ValueError) as error:
            print(f'{name}: setup failed: {error}', flush=True)
            continue
        for r in range(args.rounds):
            order = stacks if r % 2 == 0 else tuple(reversed(stacks))
            for stack in order:
                sample = run_one(name, stack, archive, out, f'r{r + 1}')
                result['samples'].append(sample)
                save()
                extra = (f" prebuilt {sample.get('prebuilt_hits')} miss {sample.get('depcache_misses')} {sample.get('toolchain')}"
                         if stack == 'remote' else '')
                print(f"{name} r{r + 1} {stack} {sample['wall']:.1f}s "
                      f"{'ok' if not sample['errors'] else sample['errors']}{extra} {sample.get('run_id') or ''}", flush=True)
        # Deleted only after the repo's last sample: deleting a checkout
        # while the next one syncs used to fail that next hosted build.
        if not args.keep:
            for sample in result['samples']:
                if sample['repo'] == name:
                    shutil.rmtree(sample['checkout'], ignore_errors=True)

    result['finished'] = time.strftime('%Y%m%d-%H%M%S')
    save()
    print(json.dumps(result['summary'], indent=1))
    print(f'result {result_path}')
    try:
        import suite
        suite.upload_result(result_path)
    except Exception as error:  # recording is best effort
        print(f'not uploaded: {error}', file=sys.stderr)
    return 0


if __name__ == '__main__':
    sys.exit(main())
