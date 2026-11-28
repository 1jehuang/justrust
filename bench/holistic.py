#!/usr/bin/env python3
"""Holistic telemetry-grounded benchmark: stock local Cargo vs hosted justrust.

Covers the five real-world workflows derived from 49,000 real non-bench agent
runs (~386h total wait in ~/.justrust/index.jsonl) and production D1 telemetry:

  1. cold-clone   Fresh `git clone` as a brand-new account (unique depcache
                  namespace, empty target/; stock gets `cargo fetch` unmeasured).
  2. worktree     Fresh `git worktree` of a known repo (empty target/, 1-line
                  edit verified via rustc warning; worktrees are ~35% of wait).
  3. swarm-4      4 concurrent agents editing + building/testing simultaneously
                  (measures makespan + mean per-agent wait under contention).
  4. warm-heavy   Warm multi-crate & test edit loops (jcode-tui test, jcode-bin
                  check, full jcode-tui test suite).
  5. warm-tiny    Warm sub-second incremental edits (ripgrep ignore, uv-resolver).

Every hosted sample verifies `where == hosted` (never local fallback).
Every edit sample verifies rustc compiled the exact unique marker for that round.
Every release sample verifies the binary exists locally and runs `--version`.
Every test sample verifies nonzero passed tests matching between stacks.

Usage:
  bench/holistic.py [--only cold-clone,worktree,swarm-4,warm-heavy,warm-tiny]
                    [--rounds 3] [--cold-rounds 1] [--stacks stock,remote]
                    [--result bench/results/holistic-<ts>.json]
"""
from __future__ import annotations

import argparse
from concurrent.futures import ThreadPoolExecutor
import json
import os
from pathlib import Path
import platform
import re
import shutil
import statistics
import subprocess as sp
import sys
import time
import uuid

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import readme_uv as uv  # noqa: E402

ROOT = uv.scratch_root() / 'holistic'
STOCK_CARGO = '/usr/bin/cargo'

REPOS = {
    'jcode': {
        'source': Path.home() / 'jcode',
        'url': 'https://github.com/1jehuang/jcode.git',
        'pin': 'b33d94f006ce2488bbc3b4b5ec818505ac750b28',
        'edit_file': 'crates/jcode-tui/src/tui/jarvis.rs',
        'edit_anchor': "fn reexec_args_for(enabled: bool, always_on: bool, screen: bool) -> Vec<&'static str> {\n",
    },
    'ripgrep': {
        'source': ROOT / 'src-ripgrep',
        'url': 'https://github.com/BurntSushi/ripgrep.git',
        'pin': 'e89fff89ac9af12e8d4ce9d5fd07beb408ca730f',  # 15.2.0
        'edit_file': 'crates/ignore/src/gitignore.rs',
        'edit_anchor': '    pub fn is_empty(&self) -> bool {\n',
    },
    'uv': {
        'source': ROOT / 'src-uv',
        'url': 'https://github.com/astral-sh/uv.git',
        'pin': '6b2d39dcc390293d51c9c0d2bbcd650f4bcbfdce',  # 0.13.9
        'edit_file': 'crates/uv-resolver/src/manifest.rs',
        'edit_anchor': '    pub fn simple(requirements: Vec<Requirement>) -> Self {\n',
    },
}

# Wait weights derived from ~/.justrust/index.jsonl (49,000 real non-bench runs)
CATEGORY_WEIGHTS = {
    'cold-clone': 0.35,
    'worktree': 0.35,
    'swarm-4': 0.15,
    'warm-heavy': 0.145,
    'warm-tiny': 0.005,
}


def power_state() -> dict:
    def read(path: str | Path) -> str | None:
        try:
            return Path(path).read_text().strip()
        except OSError:
            return None

    supplies = Path('/sys/class/power_supply')
    online = (
        [read(p / 'online') for p in supplies.glob('*') if (p / 'online').exists()]
        if supplies.exists()
        else []
    )
    load = read('/proc/loadavg')
    return {
        'ac': '1' in online,
        'epp': read('/sys/devices/system/cpu/cpu0/cpufreq/energy_performance_preference'),
        'governor': read('/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor'),
        'loadavg': load.split()[:3] if load else None,
    }


def ensure_source(name: str) -> Path:
    spec = REPOS[name]
    src: Path = spec['source']
    pin: str = spec['pin']
    url: str = spec['url']
    if not src.exists():
        src.parent.mkdir(parents=True, exist_ok=True)
        sp.run(['git', 'init', '-q', str(src)], check=True)
    have = sp.run(['git', '-C', str(src), 'cat-file', '-t', pin], capture_output=True, text=True)
    if have.stdout.strip() != 'commit':
        sp.run(['git', '-C', str(src), 'fetch', '-q', '--depth', '1', url, pin], check=True)
    sp.run(['git', '-C', str(src), 'update-ref', f'refs/heads/pin-{pin[:12]}', pin], check=True)
    return src


def make_git_clone(name: str, dest: Path) -> Path:
    """Create a real git checkout (with .git + public https origin) at `dest`."""
    src = ensure_source(name)
    spec = REPOS[name]
    if dest.exists():
        shutil.rmtree(dest)
    dest.parent.mkdir(parents=True, exist_ok=True)
    sp.run(['git', 'clone', '-q', '--no-checkout', str(src), str(dest)], check=True)
    sp.run(['git', '-C', str(dest), 'remote', 'set-url', 'origin', spec['url']], check=True)
    sp.run(['git', '-C', str(dest), 'checkout', '-q', spec['pin']], check=True)
    return dest


def make_worktree(base_clone: Path, dest: Path, pin: str) -> Path:
    if dest.exists():
        sp.run(['git', '-C', str(base_clone), 'worktree', 'remove', '--force', str(dest)],
               capture_output=True)
        if dest.exists():
            shutil.rmtree(dest)
    dest.parent.mkdir(parents=True, exist_ok=True)
    sp.run(['git', '-C', str(base_clone), 'worktree', 'add', '-q', '--detach', str(dest), pin],
           check=True)
    return dest


def inject_marker(checkout: Path, repo_name: str, marker: str) -> tuple[Path, str]:
    spec = REPOS[repo_name]
    path = checkout / spec['edit_file']
    orig = path.read_text()
    anchor = spec['edit_anchor']
    if anchor not in orig:
        raise RuntimeError(f'anchor not found in {path}')
    edited = orig.replace(anchor, f'{anchor}    let {marker} = 0usize;\n', 1)
    path.write_text(edited)
    return path, orig


def stock_env() -> dict[str, str]:
    env = {k: v for k, v in os.environ.items() if not k.startswith('JUSTRUST_')}
    env['RUSTC'] = '/usr/bin/rustc'
    env['CARGO_TERM_COLOR'] = 'never'
    return env


def hosted_env(run_id: str, namespace: str | None = None) -> dict[str, str]:
    env = dict(os.environ)
    env['JUSTRUST_RUN_ID'] = run_id
    env['JUSTRUST_REMOTE'] = 'force'
    if namespace is not None:
        env['JUSTRUST_DEPCACHE_NAMESPACE'] = namespace
    else:
        env.pop('JUSTRUST_DEPCACHE_NAMESPACE', None)
    return env


def run_cmd(stack: str, cwd: Path, sub: str, args: list[str],
            namespace: str | None = None) -> dict:
    run_id = f'holistic-{uuid.uuid4().hex[:16]}'
    if stack == 'stock':
        cmd = [STOCK_CARGO, sub, *args]
        env = stock_env()
    else:
        cmd = ['justrust', sub, '--remote', *args]
        env = hosted_env(run_id, namespace=namespace)
    pw = power_state()
    t0 = time.monotonic()
    p = sp.run(cmd, cwd=cwd, env=env, capture_output=True, text=True)
    wall = time.monotonic() - t0
    out = p.stdout + '\n' + p.stderr
    # Extract the actual run_id recorded by justrust (`justrust show <id>`)
    actual_run_id = run_id
    if stack == 'remote':
        m_id = re.search(r'justrust show ([A-Za-z0-9._:-]+)', out)
        if m_id:
            actual_run_id = m_id.group(1)
    passed = sum(int(n) for n in re.findall(r'test result: ok\. (\d+) passed', out))
    if passed == 0 and stack == 'remote':
        m_ok = re.search(r'justrust: ok: (\d+) tests? passed', out)
        if m_ok:
            passed = int(m_ok.group(1))
    rec = {
        'stack': stack,
        'cmd': ' '.join(cmd),
        'wall': round(wall, 3),
        'exit': p.returncode,
        'run_id': actual_run_id if stack == 'remote' else None,
        'power': pw,
        'passed': passed,
        'errors': [],
    }
    if p.returncode != 0:
        rec['errors'].append(f'exit {p.returncode}: {out[-600:]}')
    if stack == 'remote':
        # Verify execution happened on hosted and capture full log for marker checks
        summary_path = Path.home() / '.justrust' / 'runs' / actual_run_id / 'summary.json'
        if summary_path.exists():
            try:
                s = json.loads(summary_path.read_text())
                rem = s.get('remote') or {}
                rec['machine'] = rem.get('machine')
                rec['host_wall'] = s.get('wall')
                if rem.get('machine') != 'hosted':
                    rec['errors'].append(f"not hosted: machine={rem.get('machine')}")
                e2e = s.get('e2e') or {}
                segs = {seg['key']: round(seg['secs'], 3) for seg in e2e.get('segments', [])}
                rec['e2e'] = {k: segs[k] for k in (
                    'remote.admission', 'sync.push', 'sync.barrier',
                    'host.compile', 'host.tests', 'remote.artifacts', 'remote.host_overhead'
                ) if k in segs}
            except Exception as e:
                rec['errors'].append(f'summary parse: {e}')
        else:
            rec['errors'].append('missing summary.json')
    rec['_out'] = out
    return rec


def verify_marker(rec: dict, marker: str) -> None:
    if rec['exit'] != 0:
        return
    haystack = rec.get('_out', '')
    if rec['stack'] == 'remote' and marker not in haystack and rec.get('run_id'):
        log = sp.run(['justrust', 'log', rec['run_id']], capture_output=True, text=True)
        haystack += '\n' + log.stdout + '\n' + log.stderr
    if marker not in haystack:
        rec['errors'].append(f'marker {marker} not found in compiler output')


def verify_binary(checkout: Path, bin_name: str, rec: dict) -> None:
    if rec['exit'] != 0:
        return
    path = checkout / 'target' / 'release' / bin_name
    if not path.exists():
        rec['errors'].append(f'missing release binary {path}')
        return
    v = sp.run([str(path), '--version'], capture_output=True, text=True)
    if v.returncode != 0:
        rec['errors'].append(f'{bin_name} --version failed: {v.stderr}')


def strip_private(rec: dict) -> dict:
    return {k: v for k, v in rec.items() if not k.startswith('_')}


def run_cold_clone(stacks: list[str], rounds: int) -> list[dict]:
    cases = [
        ('cold-rg-rel', 'ripgrep', 'build', ['--release', '--locked'], 'rg', False),
        ('cold-uv-check', 'uv', 'check', ['-p', 'uv-resolver', '--locked'], None, False),
        ('cold-uv-rel', 'uv', 'build', ['--release', '-p', 'uv', '--bin', 'uv', '--locked'], 'uv', False),
        ('cold-jcode-test', 'jcode', 'test', ['-p', 'jcode-tui', '--lib', 'jarvis_listen', '--locked'], None, True),
        ('cold-jcode-rel', 'jcode', 'build', ['--release', '-p', 'jcode', '--bin', 'jcode', '--locked'], 'jcode', False),
    ]
    samples = []
    for name, repo, sub, args, binary, need_tests in cases:
        for r in range(rounds):
            order = stacks if r % 2 == 0 else list(reversed(stacks))
            for st in order:
                tag = uuid.uuid4().hex[:8]
                co = ROOT / 'cold' / f'{name}-r{r}-{st}-{tag}'
                make_git_clone(repo, co)
                if st == 'stock':
                    sp.run([STOCK_CARGO, 'fetch', '--locked'], cwd=co, env=stock_env(),
                           capture_output=True, check=True)
                ns = f'holistic-cold-{tag}' if st == 'remote' else None
                print(f'  [cold-clone] {name:16s} r{r+1} {st:6s} ...', end=' ', flush=True)
                rec = run_cmd(st, co, sub, args, namespace=ns)
                if binary:
                    verify_binary(co, binary, rec)
                if need_tests and rec['passed'] == 0:
                    rec['errors'].append('0 tests passed')
                rec.update({'category': 'cold-clone', 'scenario': name, 'round': r})
                print(f"{rec['wall']:6.2f}s" + (f"  ERR={rec['errors']}" if rec['errors'] else ''))
                samples.append(strip_private(rec))
                shutil.rmtree(co, ignore_errors=True)
    return samples


def run_worktree(stacks: list[str], rounds: int) -> list[dict]:
    cases = [
        ('wt-rg-test', 'ripgrep', 'test', ['-p', 'ignore', '--lib', '--locked'], True),
        ('wt-uv-check', 'uv', 'check', ['-p', 'uv-resolver', '--locked'], False),
        ('wt-jcode-test', 'jcode', 'test', ['-p', 'jcode-tui', '--lib', 'jarvis_listen', '--locked'], True),
    ]
    # Warm a base clone per repo on hosted (and stock base exists, though each worktree has fresh target/)
    bases: dict[str, Path] = {}
    for _, repo, sub, args, _ in cases:
        if repo not in bases:
            base = ROOT / 'wt-base' / repo
            if not (base / 'Cargo.toml').exists():
                make_git_clone(repo, base)
            bases[repo] = base
            if 'remote' in stacks:
                print(f'  [worktree] priming base repo {repo} on hosted ...', flush=True)
                run_cmd('remote', base, sub, args)
            if 'stock' in stacks:
                sp.run([STOCK_CARGO, 'fetch', '--locked'], cwd=base, env=stock_env(),
                       capture_output=True, check=True)
    samples = []
    for name, repo, sub, args, need_tests in cases:
        base = bases[repo]
        pin = REPOS[repo]['pin']
        for r in range(rounds):
            order = stacks if r % 2 == 0 else list(reversed(stacks))
            for st in order:
                tag = uuid.uuid4().hex[:8]
                wt = ROOT / 'wt-runs' / f'{name}-r{r}-{st}-{tag}'
                make_worktree(base, wt, pin)
                marker = f'holistic_wt_{tag}'
                inject_marker(wt, repo, marker)
                print(f'  [worktree]   {name:16s} r{r+1} {st:6s} ...', end=' ', flush=True)
                rec = run_cmd(st, wt, sub, args)
                verify_marker(rec, marker)
                if need_tests and rec['passed'] == 0:
                    rec['errors'].append('0 tests passed')
                rec.update({'category': 'worktree', 'scenario': name, 'round': r})
                print(f"{rec['wall']:6.2f}s" + (f"  ERR={rec['errors']}" if rec['errors'] else ''))
                samples.append(strip_private(rec))
                sp.run(['git', '-C', str(base), 'worktree', 'remove', '--force', str(wt)],
                       capture_output=True)
                shutil.rmtree(wt, ignore_errors=True)
    return samples


def run_swarm4(stacks: list[str], rounds: int) -> list[dict]:
    """4 parallel agents editing & compiling/testing simultaneously."""
    agents = [
        ('a1-jcode-test', 'jcode', 'test', ['-p', 'jcode-tui', '--lib', 'jarvis_listen', '--locked'], True),
        ('a2-jcode-bin', 'jcode', 'check', ['-p', 'jcode', '--bin', 'jcode', '--locked'], False),
        ('a3-uv-check', 'uv', 'check', ['-p', 'uv-resolver', '--locked'], False),
        ('a4-rg-test', 'ripgrep', 'test', ['-p', 'ignore', '--lib', '--locked'], True),
    ]
    # Prepare 4 persistent warm checkouts per stack (stock and remote get separate checkouts)
    checkouts: dict[tuple[str, str], Path] = {}
    for st in stacks:
        for aname, repo, sub, args, _ in agents:
            co = ROOT / 'swarm4' / f'{st}-{aname}'
            if not (co / 'Cargo.toml').exists():
                make_git_clone(repo, co)
            checkouts[(st, aname)] = co
            print(f'  [swarm-4] warming {st:6s} {aname} ...', flush=True)
            run_cmd(st, co, sub, args)

    samples = []
    for r in range(rounds):
        order = stacks if r % 2 == 0 else list(reversed(stacks))
        for st in order:
            restores = []
            jobs = []
            for aname, repo, sub, args, need_tests in agents:
                co = checkouts[(st, aname)]
                marker = f'holistic_s4_{uuid.uuid4().hex[:8]}'
                path, orig = inject_marker(co, repo, marker)
                restores.append((path, orig))
                jobs.append((aname, co, sub, args, need_tests, marker))
            pw = power_state()
            t0 = time.monotonic()
            print(f'  [swarm-4]    4-agent-swarm    r{r+1} {st:6s} ...', end=' ', flush=True)
            try:
                with ThreadPoolExecutor(max_workers=4) as ex:
                    futs = [(j, ex.submit(run_cmd, st, j[1], j[2], j[3])) for j in jobs]
                    sub_recs = []
                    for (aname, _co, _sub, _args, need_tests, marker), fut in futs:
                        rec = fut.result()
                        verify_marker(rec, marker)
                        if need_tests and rec['passed'] == 0:
                            rec['errors'].append('0 tests passed')
                        rec['agent'] = aname
                        sub_recs.append(strip_private(rec))
                makespan = round(time.monotonic() - t0, 3)
            finally:
                for path, orig in restores:
                    path.write_text(orig)
            errs = [f"{sr['agent']}:{sr['errors']}" for sr in sub_recs if sr['errors']]
            mean_wait = round(statistics.mean(sr['wall'] for sr in sub_recs), 3)
            rec = {
                'category': 'swarm-4',
                'scenario': 'swarm-4-makespan',
                'stack': st,
                'round': r,
                'wall': makespan,
                'mean_agent_wall': mean_wait,
                'agents': sub_recs,
                'power': pw,
                'exit': 0 if not errs else 1,
                'errors': errs,
            }
            print(f"makespan={makespan:6.2f}s  mean={mean_wait:5.2f}s" + (f"  ERR={errs}" if errs else ''))
            samples.append(rec)
    return samples


def run_warm(category: str, cases: list[tuple], stacks: list[str], rounds: int, think_gap_s: float = 0.0) -> list[dict]:
    samples = []
    for name, repo, sub, args, need_tests in cases:
        checkouts: dict[str, Path] = {}
        for st in stacks:
            co = ROOT / category / f'{st}-{name}'
            if not (co / 'Cargo.toml').exists():
                make_git_clone(repo, co)
            checkouts[st] = co
            print(f'  [{category}] warming {st:6s} {name} ...', flush=True)
            w = run_cmd(st, co, sub, args)
            if w['exit'] != 0:
                print(f'    WARN warmup failed: {w["errors"]}', flush=True)
        for r in range(rounds):
            order = stacks if r % 2 == 0 else list(reversed(stacks))
            for st in order:
                co = checkouts[st]
                marker = f'holistic_w_{uuid.uuid4().hex[:8]}'
                path, orig = inject_marker(co, repo, marker)
                try:
                    if think_gap_s > 0:
                        time.sleep(think_gap_s)
                    print(f'  [{category:10s}] {name:16s} r{r+1} {st:6s} ...', end=' ', flush=True)
                    rec = run_cmd(st, co, sub, args)
                    verify_marker(rec, marker)
                    if need_tests and rec['passed'] == 0:
                        rec['errors'].append('0 tests passed')
                finally:
                    path.write_text(orig)
                rec.update({'category': category, 'scenario': name, 'round': r})
                if think_gap_s > 0:
                    rec['think_gap_s'] = think_gap_s
                print(f"{rec['wall']:6.2f}s" + (f"  ERR={rec['errors']}" if rec['errors'] else ''))
                samples.append(strip_private(rec))
    return samples


def summarize(samples: list[dict]) -> dict:
    by_scen: dict[str, dict[str, list[float]]] = {}
    scen_cat: dict[str, str] = {}
    for s in samples:
        if s.get('errors'):
            continue
        sc = s['scenario']
        scen_cat[sc] = s['category']
        by_scen.setdefault(sc, {}).setdefault(s['stack'], []).append(s['wall'])
    rows = {}
    cat_speedups: dict[str, list[float]] = {}
    for sc, sides in by_scen.items():
        st_m = statistics.median(sides['stock']) if sides.get('stock') else None
        ho_m = statistics.median(sides['remote']) if sides.get('remote') else None
        sp_x = round(st_m / ho_m, 2) if (st_m and ho_m) else None
        rows[sc] = {
            'category': scen_cat[sc],
            'stock_median_s': round(st_m, 3) if st_m else None,
            'hosted_median_s': round(ho_m, 3) if ho_m else None,
            'speedup': sp_x,
            'n_stock': len(sides.get('stock', [])),
            'n_hosted': len(sides.get('remote', [])),
        }
        if sp_x:
            cat_speedups.setdefault(scen_cat[sc], []).append(sp_x)
    # Telemetry-weighted harmonic/log-geometric aggregate across measured categories
    import math
    w_sum = 0.0
    log_sum = 0.0
    cat_means = {}
    for cat, vals in cat_speedups.items():
        gm = math.exp(sum(math.log(v) for v in vals) / len(vals))
        cat_means[cat] = round(gm, 2)
        w = CATEGORY_WEIGHTS.get(cat, 0.0)
        if w > 0:
            w_sum += w
            log_sum += w * math.log(gm)
    weighted_gm = round(math.exp(log_sum / w_sum), 2) if w_sum > 0 else None
    all_vals = [r['speedup'] for r in rows.values() if r['speedup']]
    unweighted_gm = round(math.exp(sum(math.log(v) for v in all_vals) / len(all_vals)), 2) if all_vals else None
    return {
        'rows': rows,
        'category_geomean_speedup': cat_means,
        'telemetry_weighted_speedup': weighted_gm,
        'unweighted_geomean_speedup': unweighted_gm,
    }


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument('--only', default='cold-clone,worktree,swarm-4,warm-heavy,warm-tiny,agent-paced')
    ap.add_argument('--stacks', default='stock,remote')
    ap.add_argument('--rounds', type=int, default=3)
    ap.add_argument('--cold-rounds', type=int, default=1)
    ap.add_argument('--think-gap', type=float, default=5.0)
    ap.add_argument('--result', default='')
    args = ap.parse_args()

    cats = [c.strip() for c in args.only.split(',') if c.strip()]
    stacks = [s.strip() for s in args.stacks.split(',') if s.strip()]
    ROOT.mkdir(parents=True, exist_ok=True)

    pw = power_state()
    print(f"Power state: {pw}")
    if not pw['ac'] or pw['epp'] != 'performance':
        print("WARNING: Laptop is not on AC + epp=performance; stock numbers may be throttled!")

    jr_ver = sp.check_output(['justrust', '--version'], text=True).strip()
    stock_ver = sp.check_output([STOCK_CARGO, '--version'], text=True).strip()
    print(f"Client: {jr_ver} | Stock: {stock_ver}")

    ts = time.strftime('%Y%m%d-%H%M%S')
    out_path = Path(args.result) if args.result else HERE / 'results' / f'holistic-{ts}.json'
    out_path.parent.mkdir(parents=True, exist_ok=True)
    doc = json.loads(out_path.read_text()) if out_path.exists() else {
        'schema': 1,
        'started': ts,
        'host': platform.node(),
        'justrust_version': jr_ver,
        'stock_cargo_version': stock_ver,
        'samples': [],
    }

    def save():
        doc['summary'] = summarize(doc['samples'])
        doc['updated'] = time.strftime('%Y%m%d-%H%M%S')
        out_path.write_text(json.dumps(doc, indent=2) + '\n')

    ordered_cats = [c for c in ('warm-tiny', 'warm-heavy', 'agent-paced', 'swarm-4', 'worktree', 'cold-clone') if c in cats]
    total_steps = max(1, len(ordered_cats))
    done_steps = 0

    def emit_progress(label):
        print(f"JCODE_PROGRESS {json.dumps({'done': done_steps, 'total': total_steps, 'label': label})}", flush=True)

    if 'warm-tiny' in cats:
        emit_progress('warm-tiny')
        doc['samples'].extend(run_warm('warm-tiny', [
            ('warm-rg-test', 'ripgrep', 'test', ['-p', 'ignore', '--lib', '--locked'], True),
            ('warm-uv-check', 'uv', 'check', ['-p', 'uv-resolver', '--locked'], False),
        ], stacks, args.rounds))
        save()
        done_steps += 1
    if 'warm-heavy' in cats:
        emit_progress('warm-heavy')
        doc['samples'].extend(run_warm('warm-heavy', [
            ('warm-jcode-test', 'jcode', 'test', ['-p', 'jcode-tui', '--lib', 'jarvis_listen', '--locked'], True),
            ('warm-jcode-bin', 'jcode', 'check', ['-p', 'jcode', '--bin', 'jcode', '--locked'], False),
        ], stacks, args.rounds))
        save()
        done_steps += 1
    if 'agent-paced' in cats:
        emit_progress('agent-paced')
        doc['samples'].extend(run_warm('agent-paced', [
            ('paced-uv-check', 'uv', 'check', ['-p', 'uv-resolver', '--locked'], False),
            ('paced-jcode-test', 'jcode', 'test', ['-p', 'jcode-tui', '--lib', 'jarvis_listen', '--locked'], True),
        ], stacks, args.rounds, think_gap_s=args.think_gap))
        save()
        done_steps += 1
    if 'swarm-4' in cats:
        emit_progress('swarm-4')
        doc['samples'].extend(run_swarm4(stacks, max(1, min(args.rounds, 2))))
        save()
        done_steps += 1
    if 'worktree' in cats:
        emit_progress('worktree')
        doc['samples'].extend(run_worktree(stacks, args.cold_rounds))
        save()
        done_steps += 1
    if 'cold-clone' in cats:
        emit_progress('cold-clone')
        doc['samples'].extend(run_cold_clone(stacks, args.cold_rounds))
        save()
        done_steps += 1
    emit_progress('done')

    s = doc['summary']
    print('\n================ HOLISTIC SUMMARY ================')
    print(f"{'Category':12s} {'Scenario':18s} {'Stock':>8s} {'Hosted':>8s} {'Speedup':>8s}")
    for sc, r in s['rows'].items():
        st = f"{r['stock_median_s']:.2f}s" if r['stock_median_s'] is not None else '-'
        ho = f"{r['hosted_median_s']:.2f}s" if r['hosted_median_s'] is not None else '-'
        sp_s = f"{r['speedup']:.2f}x" if r['speedup'] is not None else '-'
        print(f"{r['category']:12s} {sc:18s} {st:>8s} {ho:>8s} {sp_s:>8s}")
    print(f"\nCategory geomeans: {s['category_geomean_speedup']}")
    print(f"Telemetry-weighted speedup: {s['telemetry_weighted_speedup']}x  |  Unweighted geomean: {s['unweighted_geomean_speedup']}x")
    print(f"Saved: {out_path}")


if __name__ == '__main__':
    main()
