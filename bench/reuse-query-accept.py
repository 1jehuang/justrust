#!/usr/bin/env python3
"""Direct rustc incremental correctness acceptance, not a performance benchmark."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys

FLAG = '-Zjr-cache-small-query-values'
SELECTED = ('hir_owner_parent_q', 'constness', 'coroutine_kind')
BASE = '''#![allow(dead_code)]
extern crate retention_dep;
const fn value() -> u32 { 7 }
fn ordinary() -> u32 { let closure = || { fn nested() -> u32 { 2 } nested() }; closure() }
async fn asynchronous() -> u32 { 3 }
macro_rules! amount { () => { 4u32 }; }
fn main() { println!("{}", value() + ordinary() + amount!() + retention_dep::number(1)); }
'''
DEP = 'pub fn number(x: u32) -> u32 { x + 1 }\n'


def cases():
    """Ordered edits intentionally reuse each mode's incremental directories."""
    return [
        ('initial', BASE, DEP, '15\n'),
        ('unchanged', BASE, DEP, '15\n'),
        ('body-edit', BASE.replace('{ 7 }', '{ 8 }'), DEP, '16\n'),
        ('body-revert', BASE, DEP, '15\n'),
        ('nonconst', BASE.replace('const fn value', 'fn value'), DEP, '15\n'),
        ('const-recovery', BASE, DEP, '15\n'),
        ('ordinary-from-async', BASE.replace('async fn asynchronous', 'fn asynchronous'), DEP, '15\n'),
        ('async-recovery', BASE, DEP, '15\n'),
        ('async-closure', BASE.replace('let closure = ||', 'let unused = async || 9u32; let closure = ||'), DEP, '15\n'),
        ('nested-owner-add', BASE.replace('nested() };', 'fn extra() -> u32 { 1 } nested() + extra() };'), DEP, '16\n'),
        ('owner-remove', BASE.replace('async fn asynchronous() -> u32 { 3 }\n', ''), DEP, '15\n'),
        ('owner-add', BASE + '\nmod added { pub const fn item() -> u32 { 12 } }\n', DEP, '15\n'),
        ('owner-reorder', '\n'.join(BASE.splitlines()[:2] + [BASE.splitlines()[4], BASE.splitlines()[3], BASE.splitlines()[2]] + BASE.splitlines()[5:]) + '\n', DEP, '15\n'),
        ('macro-edit', BASE.replace('4u32', '5u32'), DEP, '16\n'),
        ('macro-revert', BASE, DEP, '15\n'),
        ('lint-error', '#![deny(unused_variables)]\n' + BASE.replace('let closure', 'let unused = 0; let closure'), DEP, None),
        ('lint-recovery', BASE, DEP, '15\n'),
        ('type-error', BASE.replace('{ 7 }', '{ "wrong" }'), DEP, None),
        ('type-recovery', BASE, DEP, '15\n'),
        ('dependency-signature', BASE.replace('number(1)', 'number(1u64) as u32'), DEP.replace('u32', 'u64'), '15\n'),
        ('dependency-mismatch', BASE.replace('number(1)', 'number(1u32)'), DEP.replace('u32', 'u64'), None),
        ('dependency-recovery', BASE, DEP, '15\n'),
    ]


def digest(path):
    h = hashlib.sha256()
    with path.open('rb') as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b''):
            h.update(chunk)
    return h.hexdigest()


def run(command):
    try:
        p = subprocess.run([str(x) for x in command], capture_output=True, text=True, timeout=180)
    except subprocess.TimeoutExpired as error:
        raise RuntimeError('Compiler/program timed out: ' + json.dumps(dict(
            command=[str(x) for x in command],
            stdout=(error.stdout or b'').decode(errors='replace') if isinstance(error.stdout, bytes) else error.stdout,
            stderr=(error.stderr or b'').decode(errors='replace') if isinstance(error.stderr, bytes) else error.stderr,
        ), indent=2)) from error
    return dict(command=[str(x) for x in command], status=p.returncode, stdout=p.stdout, stderr=p.stderr)


def normalized(result, source_dir):
    # No whitespace, diagnostic-code, hash, line-number or warning suppression.
    return {k: result[k].replace(str(source_dir), '<SOURCE>') if k in ('stdout', 'stderr') else result[k]
            for k in ('status', 'stdout', 'stderr')}


def compare(off, on, off_dir, on_dir, label):
    if normalized(off, off_dir) != normalized(on, on_dir):
        raise AssertionError(f'{label}: flag changed status/diagnostics/output\n' + json.dumps(dict(off=off, on=on), indent=2))


def capture(rustc, root):
    root.mkdir(mode=0o700, parents=False, exist_ok=False)
    report = dict(schema=1, scope='query-value-retention-correctness-only-no-speedup-claim',
                  flag=FLAG, verify_ich=True, verdict='incomplete', planned_cases=len(cases()),
                  completed_cases=0, observations=[])
    try:
        version = run([rustc, '-vV'])
        report['compiler'] = dict(path=str(rustc), sha256=digest(rustc), version=version)
        sysroot = run([rustc, '--print', 'sysroot'])
        report['sysroot_query'] = sysroot
        if version['status'] or sysroot['status']:
            raise AssertionError('Compiler identity query failed: ' + json.dumps(report, indent=2))
        drivers = sorted((Path(sysroot['stdout'].strip()) / 'lib').glob('*rustc_driver*'))
        if not drivers:
            raise AssertionError('No rustc_driver library found in compiler sysroot/lib')
        report['drivers'] = [dict(path=str(p.resolve()), sha256=digest(p)) for p in drivers if p.is_file()]
        for mode in ('off', 'on'):
            (root / mode).mkdir(mode=0o700)
        for name, source, dep, expected in cases():
            observation = dict(name=name, expected_stdout=expected, source=source, dependency_source=dep, modes={})
            report['observations'].append(observation)
            for mode in ('off', 'on'):
                work = root / mode
                (work / 'main.rs').write_text(source)
                (work / 'dep.rs').write_text(dep)
                flags = ['--edition=2024', '-Zincremental-verify-ich', '-Cdebuginfo=0']
                if mode == 'on':
                    flags.append(FLAG)
                lib = work / 'libretention_dep.rlib'
                binary = work / 'fixture'
                record = {}
                observation['modes'][mode] = record
                record['dependency'] = run([rustc, work / 'dep.rs', '--crate-name=retention_dep', '--crate-type=rlib', '-C', f'incremental={work / "dep-incremental"}', '-o', lib] + flags)
                if record['dependency']['status']:
                    raise AssertionError('Dependency failed: ' + json.dumps(record, indent=2))
                record['compile'] = run([rustc, work / 'main.rs', '--crate-name=retention_fixture', '--extern', f'retention_dep={lib}', '-C', f'incremental={work / "main-incremental"}', '-o', binary] + flags)
                if record['compile']['status'] == 0:
                    record['artifact_sha256'] = digest(binary)
                    record['runtime'] = run([binary])
            off, on = (observation['modes'][m] for m in ('off', 'on'))
            for phase in ('dependency', 'compile'):
                compare(off[phase], on[phase], root / 'off', root / 'on', f'{name}/{phase}')
            if expected is None:
                if off['compile']['status'] != 1:
                    raise AssertionError(f'{name}: expected ordinary compiler failure (1), got ' + json.dumps(observation, indent=2))
            else:
                if off['compile']['status'] != 0:
                    raise AssertionError(f'{name}: unexpected compiler failure\n' + json.dumps(observation, indent=2))
                if {k: off['runtime'][k] for k in ('status', 'stdout', 'stderr')} != {k: on['runtime'][k] for k in ('status', 'stdout', 'stderr')}:
                    raise AssertionError(f'{name}: flag changed runtime behavior\n' + json.dumps(observation, indent=2))
                if off['runtime']['status'] or off['runtime']['stdout'] != expected or off['runtime']['stderr']:
                    raise AssertionError(f'{name}: incorrect runtime result\n' + json.dumps(observation, indent=2))
            report['completed_cases'] += 1
        report['verdict'] = 'passed'
    except Exception as e:
        report['verdict'] = 'failed'
        report['error'] = str(e)
        raise
    finally:
        (root / 'acceptance.json').write_text(json.dumps(report, indent=2) + '\n')
    return report


def selected_recompute_counts(summary):
    markers = summary['native_green_reuse']['markers']
    # Missing selected-kind records are count-zero only when both native marker
    # families are observed and all their query names resolve in this capture.
    if any(markers[k]['evidence'] != 'observed' or markers[k]['records_without_query_name'] != 0
           for k in ('jr_incremental_green_query', 'jr_incremental_green_recompute')):
        raise AssertionError('Missing native marker coverage or unresolved native query names')
    counts = markers['jr_incremental_green_recompute']['query_name_interval_records']
    return {kind: counts.get(kind, 0) for kind in SELECTED}


def capture_profile_counts(rustc, root):
    """Small native count experiment, never a time-saved estimate."""
    import reuse_ceiling

    root.mkdir(mode=0o700, parents=False, exist_ok=False)
    report = dict(schema=1, scope='native-query-retention-counts-not-time-saved',
                  verdict='incomplete', verify_ich=True, allow_unmapped_event_labels=True, observations=[])
    selected = SELECTED
    source = BASE.replace('extern crate retention_dep;\n', '').replace('retention_dep::number(1)', '2u32')
    try:
        version = run([rustc, '-vV'])
        sysroot = run([rustc, '--print', 'sysroot'])
        report['compiler'] = dict(path=str(rustc), sha256=digest(rustc), version=version)
        report['sysroot_query'] = sysroot
        if version['status'] or sysroot['status']:
            raise AssertionError('Compiler identity query failed: ' + json.dumps(report, indent=2))
        drivers = sorted((Path(sysroot['stdout'].strip()) / 'lib').glob('*rustc_driver*'))
        if not drivers:
            raise AssertionError('No rustc_driver library found')
        report['drivers'] = [dict(path=str(p.resolve()), sha256=digest(p)) for p in drivers if p.is_file()]
        for name, content, expected in [('warm', source, '15\n'), ('unchanged', source, '15\n'),
                                        ('body-edit', source.replace('{ 7 }', '{ 8 }'), '16\n')]:
            observation = dict(name=name, source=content, modes={})
            report['observations'].append(observation)
            for mode in ('off', 'on'):
                work = root / mode
                work.mkdir(mode=0o700, exist_ok=True)
                fixture = work / 'fixture.rs'
                fixture.write_text(content)
                binary = work / 'fixture'
                prefix = work / name
                command = [rustc, fixture, '--crate-name=retention_profile', '--edition=2024',
                           '-C', f'incremental={work / "incremental"}', '-Zincremental-verify-ich',
                           f'{FLAG}={"yes" if mode == "on" else "no"}',
                           f'-Zself-profile={prefix}', '-Zself-profile-events=default,function-args',
                           '-o', binary]
                record = dict(compile=run(command))
                observation['modes'][mode] = record
                if record['compile']['status']:
                    raise AssertionError('Profile compilation failed: ' + json.dumps(observation, indent=2))
                record['runtime'] = run([binary])
                if (record['runtime']['status'], record['runtime']['stdout'], record['runtime']['stderr']) != (0, expected, ''):
                    raise AssertionError('Incorrect profile runtime: ' + json.dumps(observation, indent=2))
                profiles = sorted(prefix.glob('*.mm_profdata')) if prefix.is_dir() else sorted(work.glob(name + '*.mm_profdata'))
                if len(profiles) != 1:
                    raise AssertionError(f'Expected one native profile for {mode}/{name}, found {profiles}')
                raw = reuse_ceiling.query_profile.read_input(str(profiles[0]))
                events, strings = reuse_ceiling.query_profile.prepare(raw, True, False)
                summary = reuse_ceiling.summarize_records(
                    reuse_ceiling.query_profile.iter_prepared_records(events, strings, True), strings.resolve)
                record['profile'] = dict(path=str(profiles[0]), sha256=digest(profiles[0]),
                                         bytes=len(raw), summary=summary)
            off, on = (observation['modes'][m] for m in ('off', 'on'))
            compare(off['compile'], on['compile'], root / 'off', root / 'on', name)
            if name != 'warm':
                counts = {}
                for mode in ('off', 'on'):
                    counts[mode] = selected_recompute_counts(observation['modes'][mode]['profile']['summary'])
                observation['selected_green_recompute_counts'] = counts
                if any(counts['off'][kind] < counts['on'][kind] for kind in selected):
                    raise AssertionError('Selected green-recompute counts increased: ' + json.dumps(observation, indent=2))
        totals = {mode: {kind: sum(o['selected_green_recompute_counts'][mode][kind]
                                  for o in report['observations'] if o['name'] != 'warm')
                         for kind in selected} for mode in ('off', 'on')}
        report['selected_green_recompute_totals'] = totals
        if any(totals['off'][kind] <= totals['on'][kind] for kind in selected):
            raise AssertionError('Selected green-recompute totals did not all decrease: ' + json.dumps(totals, indent=2))
        report['verdict'] = 'passed'
    except Exception as error:
        report['verdict'] = 'failed'
        report['error'] = str(error)
        raise
    finally:
        (root / 'count-evidence.json').write_text(json.dumps(report, indent=2) + '\n')
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--rustc', '--compiler', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True, help='New private directory, parent must exist')
    parser.add_argument('--profile-counts', action='store_true', help='Small warm native count experiment instead of correctness matrix')
    args = parser.parse_args()
    os.umask(0o077)
    report = (capture_profile_counts if args.profile_counts else capture)(args.rustc.resolve(), args.output.resolve())
    print(json.dumps(dict(output=str(args.output), verdict=report['verdict'], cases=len(report['observations'])), indent=2))


if __name__ == '__main__':
    try:
        main()
    except Exception as error:
        print(str(error), file=sys.stderr)
        sys.exit(1)
