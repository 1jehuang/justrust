#!/usr/bin/env python3
"""Warm edit loop: stock Cargo on this machine versus justrust hosted.

Three pinned repositories, one agent-shaped command each (chosen from the
most repeated hosted calls in the run index, plus two public projects):

  jcode    test -p jcode-tui --lib jarvis    (edit crates/jcode-tui/src/tui/jarvis.rs)
  ripgrep  test -p ignore --lib --locked     (edit crates/ignore/src/gitignore.rs)
  uv       check -p uv-resolver --locked     (edit crates/uv-resolver/src/manifest.rs)

Each stack keeps its own warm state (stock: the checkout's target/, hosted:
the account's machine). Every measured sample makes a NEW edit immediately
before its command: a uniquely named unused local in a function body. No
stack can answer from a cached result of an earlier source state. Stack order
alternates per round. A sample only counts when:

  both    exit 0 and the output (hosted: `justrust log <run>`) contains
          rustc's `unused variable: warm_three_<id>` warning for THIS edit.
          That proves the exact edited source was compiled after the edit,
          so inside the measured window, whichever hosted mechanism
          compiled it (the request itself, or an edit-triggered prebuild
          the request waited for, both default product behavior)
  hosted  ran on a hosted machine (summary remote.machine)
  tests   both stacks report the same nonzero passed count

Wall time is measured here, from process start to exit: what the agent waits.

  bench/warm_three.py [--rounds 5] [--only jcode,uv]

Result: bench/results/warm-three-<time>-<rev>.json, uploaded to the
account-private record store (justrust records show bench <id>).
"""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import platform
import re
import statistics
import subprocess as sp
import sys
import tempfile
import time
import uuid

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import readme_uv as uv  # noqa: E402

ROOT = uv.scratch_root() / 'warm-three'
STOCK_CARGO = '/usr/bin/cargo'

# name, source repo, pinned commit, edited file, function header the marker
# goes after, the edited package, command
ROWS = {
    'jcode': (Path.home() / 'jcode', '557173275060be087a3763e5f448f124371b54c6',
              'crates/jcode-tui/src/tui/jarvis.rs',
              "fn reexec_args_for(enabled: bool, always_on: bool, screen: bool) -> Vec<&'static str> {\n",
              'jcode-tui', 'test -p jcode-tui --lib jarvis'),
    'ripgrep': (ROOT / 'ripgrep', '4649aa9700619f94cf9c66876e9549d83420e16c',
                'crates/ignore/src/gitignore.rs',
                '    pub fn is_empty(&self) -> bool {\n',
                'ignore', 'test -p ignore --lib --locked'),
    'uv': (uv.DEFAULT_SOURCE, uv.PIN, uv.EDIT_FILE, uv.EDIT_ANCHOR,
           'uv-resolver', 'check -p uv-resolver --locked'),
}
RIPGREP_URL = 'https://github.com/BurntSushi/ripgrep'


def checkout_for(name: str) -> Path:
    source, pin = ROWS[name][:2]
    if name == 'ripgrep' and not source.exists():
        sp.run(['git', 'clone', '-q', '--depth=1', '-b', '14.1.1', RIPGREP_URL, str(source)], check=True)
    checkout = ROOT / f'{name}-{pin[:12]}'
    if not checkout.exists():
        with tempfile.NamedTemporaryFile(dir=ROOT) as archive:
            sp.run(['git', '-C', str(source), 'archive', pin], stdout=archive, check=True)
            uv.extract_checkout(archive.name, checkout)
    return checkout


def command(stack: str, args: str) -> list[str]:
    words = args.split()
    if stack == 'stock':
        return [STOCK_CARGO, *words]
    return ['justrust', words[0], '--remote', *words[1:]]


def passed_count(text: str) -> int:
    return sum(int(n) for n in re.findall(r'test result: ok\. (\d+) passed', text))


def run_one(name: str, stack: str, checkout: Path, out: Path, tag: str, marker: str | None = None) -> dict:
    args = ROWS[name][5]
    package = ROWS[name][4]
    rid = 'warm-three-' + uuid.uuid4().hex
    env = uv.sample_environment(stack, checkout / 'target', rid)
    log = out / f'{name}-{tag}-{stack}.log'
    start = time.monotonic()
    with log.open('w') as stream:
        rc = sp.run(command(stack, args), cwd=checkout, env=env, stdout=stream, stderr=sp.STDOUT).returncode
    wall = time.monotonic() - start
    text = log.read_text(errors='replace')
    sample = {'repo': name, 'stack': stack, 'tag': tag, 'wall': wall, 'exit': rc, 'log': str(log),
              'run_id': rid if stack != 'stock' else None, 'errors': []}
    if rc:
        sample['errors'].append(f'exit {rc}')
    if stack == 'stock':
        sample['recompiled'] = bool(re.search(rf'(Compiling|Checking) {re.escape(package)} v', text))
        sample['passed'] = passed_count(text)
        sample['proof'] = proved(text, marker)
    else:
        try:
            # An attached or replayed request links its pinned id to the
            # shared run, so the summary may carry that run's id.
            summary, meta = uv.read_evidence(Path(os.environ.get('JUSTRUST_HOME', Path.home() / '.justrust')), rid)
        except (OSError, ValueError, KeyError) as error:
            sample['errors'].append(f'no run record: {error}')
            return sample
        if summary.get('exit') != 0:
            sample['errors'].append(f"summary exit {summary.get('exit')}")
        if not (summary.get('remote') or {}).get('machine'):
            sample['errors'].append('fell back locally: no remote.machine')
        sample['shared_run'] = summary.get('id') if summary.get('id') != rid else None
        units = summary.get('units') or {}
        remote = summary.get('remote') or {}
        attached = 'shared an identical build that was already running' in text
        replayed = 'before you asked' in text
        sample.update({
            'machine': remote.get('machine'), 'toolchain': summary.get('toolchain'),
            'units_compiled': units.get('compiled'), 'units_local': units.get('local'),
            'rebuild_reasons': summary.get('rebuild_reasons'),
            'attached': attached, 'replayed': replayed,
            'remote_phases': {'sync_ms': remote.get('sync_ms'), **(remote.get('run') or {})},
            'passed': (summary.get('tests') or {}).get('passed') or 0,
            'early': bool(re.search(r'early answer|rust-analyzer', text)),
        })
        sample['recompiled'] = bool((units.get('compiled') or 0) >= 1)
        full = sp.run(['justrust', 'log', summary.get('id') or rid], cwd=checkout,
                      capture_output=True, text=True).stdout
        sample['proof'] = proved(full + text, marker)
    return sample


def proved(text: str, marker: str | None) -> bool | None:
    return None if marker is None else f'unused variable: `warm_three_{marker}`' in text


def validate(sample: dict) -> None:
    if sample.get('proof') is not True:
        sample['errors'].append('no compiler warning for this edit: edited source not proven compiled')


def edit(path: Path, original: bytes, anchor: str, marker: str) -> None:
    line = f'    let warm_three_{marker} = ();\n'
    if anchor.startswith('    '):
        line = '    ' + line
    path.write_bytes(original.replace(anchor.encode(), (anchor + line).encode(), 1))


def summarize(samples: list[dict]) -> dict:
    out = {}
    for name in dict.fromkeys(s['repo'] for s in samples):
        row = {}
        for stack in ('stock', 'remote'):
            good = [s['wall'] for s in samples if s['repo'] == name and s['stack'] == stack
                    and s['tag'].startswith('r') and not s['errors']]
            bad = [s for s in samples if s['repo'] == name and s['stack'] == stack
                   and s['tag'].startswith('r') and s['errors']]
            row[stack] = {'n': len(good), 'failed': len(bad),
                          'median': statistics.median(good) if good else None,
                          'min': min(good) if good else None, 'max': max(good) if good else None}
        out[name] = row
    return out


def identity() -> dict:
    def text(cmd):
        try:
            return sp.run(cmd, capture_output=True, text=True, timeout=30).stdout.strip()
        except OSError:
            return None
    jr = text(['which', 'justrust'])
    return {'stock_cargo': text([STOCK_CARGO, '--version']), 'stock_rustc': text(['/usr/bin/rustc', '--version']),
            'justrust': text(['justrust', '--version']),
            'justrust_sha256': uv.sha256(os.path.realpath(jr)) if jr else None,
            'machine': platform.node(), 'cpu_count': os.cpu_count(),
            'cpu': next((l.split(':', 1)[1].strip() for l in open('/proc/cpuinfo') if l.startswith('model name')), None)}


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--rounds', type=int, default=5)
    parser.add_argument('--only', default=','.join(ROWS))
    args = parser.parse_args(argv)
    names = [n for n in args.only.split(',') if n]
    unknown = set(names) - set(ROWS)
    if unknown:
        parser.error(f'unknown repos: {sorted(unknown)}')
    ROOT.mkdir(parents=True, exist_ok=True)
    out = Path(tempfile.mkdtemp(dir=ROOT, prefix='results-'))
    rev = sp.run(['git', '-C', str(HERE.parent), 'rev-parse', '--short=12', 'HEAD'], capture_output=True, text=True).stdout.strip()
    stamp = time.strftime('%Y%m%d-%H%M%S')
    result_path = HERE / 'results' / f'warm-three-{stamp}-{rev}.json'
    result = {'schema': 1, 'what': 'warm edit loop, stock cargo on this machine vs justrust hosted',
              'started': stamp, 'rounds': args.rounds, 'identity': identity(), 'logs': str(out),
              'rows': {n: {'pin': ROWS[n][1], 'file': ROWS[n][2], 'command': ROWS[n][5]} for n in names},
              'samples': []}
    print(f'logs {out}', flush=True)

    def save():
        result['summary'] = summarize(result['samples'])
        result_path.parent.mkdir(exist_ok=True)
        result_path.write_text(json.dumps(result, indent=1) + '\n')

    for name in names:
        source, pin, file, anchor, _, cmd = ROWS[name]
        try:
            checkout = checkout_for(name)
        except (OSError, sp.CalledProcessError, ValueError) as error:
            print(f'{name}: setup failed: {error}', flush=True)
            continue
        path = checkout / file
        original = sp.check_output(['git', 'show', ':' + file], cwd=checkout)
        if path.read_bytes() != original or original.count(anchor.encode()) != 1:
            print(f'{name}: fixture does not match its pin, skipped', flush=True)
            continue
        try:
            # Warmup: the unedited pin on both stacks, then one edit, so each
            # stack's incremental state is current. Not measured.
            for tag in ('w0', 'w1'):
                for stack in ('stock', 'remote'):
                    if tag == 'w1':
                        edit(path, original, anchor, uuid.uuid4().hex)
                    sample = run_one(name, stack, checkout, out, tag)
                    result['samples'].append(sample)
                    print(f"{name} {tag} {stack} {sample['wall']:.2f}s {'ok' if not sample['errors'] else sample['errors']}", flush=True)
                    path.write_bytes(original)
            for r in range(args.rounds):
                order = ('stock', 'remote') if r % 2 == 0 else ('remote', 'stock')
                for stack in order:
                    marker = uuid.uuid4().hex
                    edit(path, original, anchor, marker)
                    try:
                        sample = run_one(name, stack, checkout, out, f'r{r + 1}', marker)
                    finally:
                        path.write_bytes(original)
                    validate(sample)
                    result['samples'].append(sample)
                    print(f"{name} r{r + 1} {stack} {sample['wall']:.2f}s "
                          f"{'ok' if not sample['errors'] else sample['errors']} {sample.get('run_id') or ''}", flush=True)
                    save()
                pair = [s for s in result['samples'] if s['repo'] == name and s['tag'] == f'r{r + 1}']
                if cmd.startswith('test'):
                    counts = {s['passed'] for s in pair}
                    if len(counts) != 1 or 0 in counts:
                        for s in pair:
                            s['errors'].append(f'test counts differ or zero: {sorted(counts)}')
        finally:
            path.write_bytes(original)
        save()

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
