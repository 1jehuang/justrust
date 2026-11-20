#!/usr/bin/env python3
"""Held non-test library variant of reuse-cgu-desktop.py. See reuse-cgu-library.md."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import subprocess
import tarfile
import tempfile
import time

from synth.pair_space import PairSpace
import suite

spec = importlib.util.spec_from_file_location('native_gate', Path(__file__).with_name('reuse-cgu-native-gate.py'))
gate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gate)
check = gate.check
REV = '27d2a732bb1f9ac852507f27d0611b20876fd7fd'
SRC = 'crates/jcode-base/src/config/default_file.rs'
TEST = 'crates/jcode-base/tests/menubar_config.rs'
ANCHOR = b'        default_content\n            .replace'
MARK = b'JR_HELD_LIBRARY_EDIT'


def fixture(repo):
    def blob(path):
        return subprocess.check_output(['git', '-C', repo, 'show', f'{REV}:{path}'])
    src, test = blob(SRC), blob(TEST)
    anchor = b'    assert!(contents.contains("[menubar]"));'
    check(src.count(ANCHOR) == test.count(anchor) == 1 and MARK not in src, 'held source anchor mismatch')
    edited = src.replace(ANCHOR, b'        default_content.replace("# jcode configuration file", "# JR_HELD_LIBRARY_EDIT")\n            .replace')
    probe = test.replace(anchor, anchor + b'\n    assert_eq!(contents.contains("JR_HELD_LIBRARY_EDIT"), std::env::var("CGU_EDITED").unwrap() == "1");')
    return src, edited, probe


def exposed(unit, units):
    """Post-metadata library time with no overlapping recorded rustc, not time saved."""
    start = unit['start'] + unit['rmeta_secs']
    end = unit['end']
    check(start <= end, 'invalid rmeta timestamp')
    cursor, total = start, 0.0
    for a, b in sorted((max(start, u['start']), min(end, u['end'])) for u in units if u is not unit and u['end'] > start and u['start'] < end):
        total += max(0, a - cursor)
        cursor = max(cursor, b)
    return total + max(0, end - cursor)


def read_trial(root, env, args, pin, fail=False):
    check(suite.foreign_load()[1] == 0, 'deferred: another compiler is active')
    start = time.monotonic()
    p = subprocess.run([suite.resolve_justrust_binary(env), *args], cwd=root, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    wall = time.monotonic() - start
    print(p.stdout, flush=True)
    check((p.returncode != 0) == fail, 'unexpected compile/test exit')
    ids = re.findall(r'justrust show ([\w-]+)`', p.stdout)
    check(ids, 'missing build record')
    d = Path(env.get('JUSTRUST_HOME', Path.home() / '.justrust')) / 'runs' / ids[-1]
    s = json.loads((d / 'summary.json').read_text())
    check(s['toolchain'] == pin and (s['exit'] != 0) == fail, 'compiler or exit mismatch')
    res = s['resources']
    check(res['max_foreign_rustc'] == 0 and res['avg_other_cores'] <= suite.NOISY_CORES, 'invalid sample: concurrent load')
    units = [json.loads(line) for line in (d / 'units.jsonl').read_text().splitlines()]
    return s, units, wall, p.stdout


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('repo', type=Path, help='Git object source, never edited')
    ap.add_argument('--run', action='store_true', help='Execute only on a reserved hosted bench machine')
    ap.add_argument('--compiler-pair', nargs=2, metavar=('CONTROL_ID', 'CANDIDATE_ID'))
    ap.add_argument('--floor-gib', type=int)
    ap.add_argument('--arm-gib', type=int, help='Conservative per-arm growth bound')
    ap.add_argument('--scratch', type=Path)
    ap.add_argument('--output', type=Path)
    a = ap.parse_args()
    src, edited, probe = fixture(a.repo)
    inputs = dict(revision=REV, source=SRC, consumer=TEST, sha256=[hashlib.sha256(v).hexdigest() for v in (src, edited, probe)])
    print(json.dumps(inputs), flush=True)
    if not a.run:
        return
    check(os.environ.get('JUSTRUST_BENCH_SRC') and os.environ.get('JUSTRUST_REMOTE') == '0' and
          not any(Path('/sys/class/power_supply').glob('BAT*')), 'requires reserved on-remote.sh bench host, not laptop')
    check(a.scratch and a.scratch.is_absolute() and a.scratch.is_dir() and a.output and not a.output.exists(), 'need scratch directory and new receipt path')
    pins = a.compiler_pair or []
    check(len(set(pins)) == 2 and all(re.fullmatch(r'jr-[a-f0-9]{40}', p) for p in pins), 'need two distinct immutable full compiler pins')
    arms = dict(zip(('control', 'candidate'), pins))
    home = Path(os.environ.get('JUSTRUST_HOME', Path.home() / '.justrust'))
    check(a.floor_gib and a.arm_gib, 'explicit pair disk floor and arm bound required')
    budget = PairSpace(a.floor_gib << 30, {arm: a.arm_gib << 30 for arm in arms})
    budget.validate(arms)
    def admit():
        budget.admit({arm: [a.scratch, home, Path.home() / '.cargo', a.output.resolve().parent] for arm in arms}, arms, 'held-jcode-base')
    admit()
    tc = {arm: home / 'toolchains' / pin for arm, pin in arms.items()}
    identities = {}
    for arm, path in tc.items():
        driver, = (path / 'lib').glob('librustc_driver-*.so')
        paths = [driver, path / 'bin/rustc', path / 'bin/cargo']
        if (path / 'bin/jr-cargo').exists():
            paths.append(path / 'bin/jr-cargo')
        identities[arm] = {f.name: gate.digest(f) for f in paths}
        identities[arm]['driver'] = identities[arm][driver.name]
    check(identities['candidate']['driver'] != identities['control']['driver'], 'identical compiler drivers')
    check(all(identities['control'].get(k) == identities['candidate'].get(k) for k in ('cargo', 'jr-cargo')), 'Cargo differs between compiler arms')
    rows, full, full_ids, diagnostic_ids = [], {}, {}, {}
    with tempfile.TemporaryDirectory(dir=a.scratch, prefix='jr-lib-cgu-') as temp:
        root = Path(temp) / 'source'
        root.mkdir()
        archive = Path(temp) / 'held.tar'
        subprocess.run(['git', '-C', a.repo, 'archive', '-o', archive, REV], check=True)
        with tarfile.open(archive) as tar:
            tar.extractall(root, filter='data')
        (root / TEST).write_bytes(probe)
        config = ['--locked', '--config', 'profile.test.package.jcode-base.codegen-units=256', '-p', 'jcode-base']
        for i in range(12):
            dirty = bool(i % 2 and i <= 7)
            data = edited if dirty else src
            (root / SRC).write_bytes(data)
            for arm in list(arms)[::(-1 if i % 2 else 1)]:
                admit()
                target = Path(temp) / arm
                env = {**os.environ, 'CARGO_TARGET_DIR': str(target), 'JUSTRUST_TOOLCHAIN': arms[arm],
                       'JUSTRUST_TOOLCHAIN_REQUIRED': '1', 'JUSTRUST_PASSES': 'on', 'JUSTRUST_SLOTS': '0',
                       'JUSTRUST_DEPCACHE': '0', 'JUSTRUST_TEST_REUSE': '0', 'JUSTRUST_BUILDD': '0',
                       'JUSTRUST_EARLY': '0', 'JUSTRUST_EARLY_EXIT': '0', 'CGU_EDITED': str(int(dirty)), 'CARGO_INCREMENTAL': '1'}
                if i >= 8:
                    # Only the benchmark-created temporary arm, never a user's target.
                    subprocess.run([tc[arm] / 'bin/cargo', 'clean', '-p', 'jcode-base', '--target-dir', target], cwd=root, env=env, check=True)
                s, us, wall, _ = read_trial(root, env, ['test', *config, '--test', 'menubar_config'], arms[arm])
                u, = [u for u in us if u['crate_name'] == 'jcode_base' and not u['test'] and 'lib' in u['crate_types'] and u['incremental'] and u['opt_level'] == '0' and 'link' in u['emit']]
                consumer, = [v for v in us if v['crate_name'] == 'menubar_config' and v['test']]
                roster = sorted(v['crate_name'] for v in us)
                if i:
                    minimal = ['jcode_base', 'menubar_config']
                    check(roster in (minimal, sorted([*minimal, 'jcode_provider_openrouter_runtime'])), f'round {i}: unrelated rebuild: {roster}')
                check(s['tests']['passed'] == 2 and s['tests']['failed'] == 0, 'downstream test did not pass')
                link, = [p['secs'] for p in consumer['passes'] if p['name'] == 'run_linker']
                cgu = {r['outcome']:r['count'] for r in u['reuse'] if r['query'] == '<cgu>'}
                rows.append(dict(i=i, arm=arm, units=1, source=hashlib.sha256(data).hexdigest(), id=s['id'],
                                 compile=u['wall'], wall=wall, rss_mb=u['max_rss_mb'], link=link, cgu=cgu,
                                 roster=roster, post_metadata=u['wall']-u['rmeta_secs'], exposed=exposed(u, us), diagnostics=s['diagnostics']))
                if i == 7:
                    # Check the non-inlined global definition in addition to the consumer's runtime assertion.
                    rlib, = (target / 'debug/deps').glob('libjcode_base-*.rlib')
                    symbols = subprocess.check_output(['nm', '-Pg', '--defined-only', rlib], text=True)
                    names = [line.split()[0] for line in symbols.splitlines() if re.search(r'jcode_base.*default_config_file_contents\S* T ', line)]
                    check(len(names) == 1, 'missing or ambiguous public function definition')
                    rows[-1]['export'] = names[0]
                    admit()
                    s, _, _, _ = read_trial(root, env, ['test', *config, '--lib'], arms[arm])
                    full_ids[arm] = s['id']
                    t = s['tests']; full[arm] = (t['passed'], t['failed'], t['ignored'], t['filtered_out'])
        for arm in arms:
            admit()
            (root / SRC).write_bytes(src + b'\nconst _: () = JR_HELD_CGU_UNDEFINED;\n')
            env = {**env, 'JUSTRUST_TOOLCHAIN': arms[arm], 'CARGO_TARGET_DIR': str(Path(temp) / arm)}
            s, _, _, text = read_trial(root, env, ['test', *config, '--test', 'menubar_config'], arms[arm], fail=True)
            location = f'{SRC}:{len(src.splitlines()) + 2}:15'
            check('E0425' in text and 'JR_HELD_CGU_UNDEFINED' in text and location in text, 'missing expected error, identifier or exact span')
            diagnostic_ids[arm] = s['id']
    try:
        check(all(rows[i]['roster'] == rows[i+1]['roster'] for i in range(0,24,2)), 'different rebuilt units between arms')
        check(all(rows[i]['diagnostics'] == rows[i+1]['diagnostics'] for i in range(0,24,2)), 'success diagnostics differ')
        check(all(row['cgu'].get('loaded', 0) > 0 for row in rows if 1 <= row['i'] <= 6), 'warm edit lacks prior CGU reuse')
        result = gate.evaluate(rows, [full[k] for k in arms])
        warm = result['medians']['warm']
        check(warm['candidate']['rss_mb'] <= 1.05 * warm['control']['rss_mb'], 'warm RSS regression')
    except ValueError as error:
        result = {'rejection': str(error)}
    with a.output.open('x') as out:
        json.dump(dict(inputs=inputs, pins=arms, identities=identities, diagnostic_runs=diagnostic_ids, full_runs=full_ids,
                       scope='held downstream config consumer, fresh library with primed dependencies', samples=rows, **result), out)
    suite.upload_result(a.output)
    check('rejection' not in result, result.get('rejection'))


if __name__ == '__main__':
    main()
