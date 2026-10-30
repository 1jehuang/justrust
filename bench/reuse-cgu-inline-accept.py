#!/usr/bin/env python3
"""Frozen-rustc CGU shared-inline traversal differential correctness gate."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import sys

SPEC = importlib.util.spec_from_file_location('cgu_inline_common', Path(__file__).with_name('reuse-declarative-macro-accept.py'))
H = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(H)
FLAG = '-Zjr-cgu-shared-inline-traversal'
H.FLAG = FLAG
MARKERS = ('jr_cgu_inline_shared_traversal', 'jr_cgu_inline_shared_revisit')
BASE = '''use std::sync::atomic::{AtomicUsize, Ordering};
static DROPS: AtomicUsize = AtomicUsize::new(0);
struct Owned<T>(T);
impl<T> Drop for Owned<T> { fn drop(&mut self) { DROPS.fetch_add(1, Ordering::SeqCst); } }
#[inline(always)] fn leaf() -> u32 { 2 }
#[inline(always)] fn left() -> u32 { leaf() + 1 }
#[inline(always)] fn right() -> u32 { leaf() + 2 }
#[inline(always)] fn diamond() -> u32 { left() + right() }
#[inline(always)] fn cycle_a(n: u32) -> u32 { if n == 0 { 0 } else { cycle_b(n - 1) + 1 } }
#[inline(always)] fn cycle_b(n: u32) -> u32 { if n == 0 { 0 } else { cycle_a(n - 1) + 1 } }
#[inline(always)] fn generic<T: Copy>(value: T) -> T { value }
mod a { #[unsafe(no_mangle)] #[inline(never)] pub fn root_a() -> u32 { super::diamond() + 1 } }
mod b { #[unsafe(no_mangle)] #[inline(never)] pub fn root_b() -> u32 { super::diamond() + 2 } }
mod c { #[unsafe(no_mangle)] #[inline(never)] pub fn root_c() -> u32 { super::diamond() + 3 } }
fn main() {
    { let _first = Owned(7u32); let _second = Owned(11u64); }
    println!("{} {} {} {} {} {} {}", a::root_a(), b::root_b(), c::root_c(),
             cycle_a(4), generic(7u32), generic(11u64), DROPS.load(Ordering::SeqCst));
}
'''
EXPECTED = '8 9 10 4 7 11 2\n'
DUPLICATE = '''mod duplicate_a { #[unsafe(no_mangle)] pub fn collision() -> u32 { 1 } }
mod duplicate_b { #[unsafe(no_mangle)] pub fn collision() -> u32 { 2 } }
'''


def cases():
    return [
        H.case('initial-shared-diamond', BASE, EXPECTED),
        H.case('unchanged', BASE, EXPECTED),
        H.case('shared-leaf-edit', BASE.replace('fn leaf() -> u32 { 2 }', 'fn leaf() -> u32 { 3 }'), '10 11 12 4 7 11 2\n'),
        H.case('leaf-recovery', BASE, EXPECTED),
        H.case('single-global-root-edit', BASE.replace('super::diamond() + 1', 'super::diamond() + 4'), '11 9 10 4 7 11 2\n'),
        H.case('cycle-input-edit', BASE.replace('cycle_a(4),', 'cycle_a(6),'), '8 9 10 6 7 11 2\n'),
        H.case('generic-drop-edit', BASE.replace('let _second = Owned(11u64);', 'let _second = Owned(11u64); let _third = Owned(13u16);'), '8 9 10 4 7 11 3\n'),
        H.case('duplicate-symbol-error', BASE + DUPLICATE, None),
        H.case('duplicate-symbol-recovery', BASE, EXPECTED),
        H.case('type-error', BASE.replace('fn leaf() -> u32 { 2 }', 'fn leaf() -> u32 { "wrong" }'), None),
        H.case('error-recovery', BASE, EXPECTED),
        H.case('recovery-unchanged', BASE, EXPECTED),
    ]


def configurations():
    return [dict(name=f'cgu{cgu}-incr{int(incremental)}-opt{opt}',
                 codegen_units=cgu, incremental=incremental, opt_level=opt)
            for cgu in (1, 16) for incremental in (True, False) for opt in (0, 2)]


def schedule(index, config):
    prefix = config['name']
    return [('fresh-no', False, f'{prefix}/fresh/{index}/no'),
            ('fresh-yes', True, f'{prefix}/fresh/{index}/yes'),
            ('persistent-no', False, f'{prefix}/persistent/no'),
            ('persistent-yes', True, f'{prefix}/persistent/yes'),
            ('transition-no-before', False, f'{prefix}/transition'),
            ('transition-yes', True, f'{prefix}/transition'),
            ('transition-no-after', False, f'{prefix}/transition')]


def compile_case(rustc, root, fixture, config, enabled, cache, profile=None):
    source = root / 'fixture.rs'
    binary = root / 'fixture'
    source.write_text(fixture['source'])
    binary.unlink(missing_ok=True)
    command = [rustc, source, '--crate-name=cgu_inline_accept', '--edition=2024',
               f'{FLAG}={"yes" if enabled else "no"}', '-Zincremental-verify-ich',
               f'-Ccodegen-units={config["codegen_units"]}',
               f'-Copt-level={config["opt_level"]}', '-Cdebuginfo=0', '-o', binary]
    if config['incremental']:
        command += ['-C', f'incremental={root / "incremental" / cache}']
    if profile is not None:
        profile.mkdir(parents=True, exist_ok=False)
        command += [f'-Zself-profile={profile}', '-Zself-profile-events=generic-activity']
    result = dict(compile=H.run(command))
    if result['compile']['status'] == 0:
        result['runtime'] = H.run([binary])
        result['artifact_sha256'] = H.digest(binary)
    return result


def profile_evidence(directory):
    paths = sorted(directory.glob('*.mm_profdata'))
    if len(paths) != 1:
        raise AssertionError('Expected exactly one native CGU profile')
    spec = importlib.util.spec_from_file_location('cgu_inline_profile', Path(__file__).with_name('query_profile.py'))
    q = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(q)
    raw = q.read_input(paths[0])
    events, strings = q.prepare(raw, allow_unmapped_event_labels=True)
    counts = dict.fromkeys(MARKERS, 0)
    unresolved = 0
    records = 0
    for event in q.iter_prepared_records(events, strings, allow_unmapped_event_labels=True):
        records += 1
        if not event['label_resolved']:
            unresolved += 1
        elif event['label'] in counts:
            if event['kind'] != 'GenericActivity' or event['record_type'] != 'interval':
                raise AssertionError('Unexpected CGU native marker kind/type')
            counts[event['label']] += 1
    return dict(path=str(paths[0]), sha256=H.digest(paths[0]), bytes=len(raw), markers=counts,
                records=records, unresolved_records=unresolved, metadata=q.metadata_from_strings(strings))


def validate_mechanism(observations):
    if [(o['name'], o['enabled']) for o in observations] != [('cold', False), ('cold', True), ('warm', False), ('warm', True)]:
        raise AssertionError('Incomplete CGU mechanism evidence sequence')
    totals = dict.fromkeys(MARKERS, 0)
    for observation in observations:
        counts = observation['profile']['markers']
        if not observation['enabled']:
            if any(counts.values()):
                raise AssertionError('Off control unexpectedly emitted shared traversal markers')
        else:
            if counts[MARKERS[1]] > 0 and counts[MARKERS[0]] == 0:
                raise AssertionError('Duplicate avoidance without observed shared traversal scope')
            for marker in MARKERS:
                totals[marker] += counts[marker]
    if any(count <= 0 for count in totals.values()):
        raise AssertionError('No positive native shared traversal and duplicate DFS avoidance evidence')
    return dict(marker_totals=totals, proof='positive shared traversal and suppressed duplicate LocalCopy DFS',
                limitation='Warm partition queries may be reused without markers. Counts prove native branch execution and duplicate avoidance, not a speedup.')


def mechanism_evidence(rustc, root, evidence):
    config = dict(name='mechanism-cgu1', codegen_units=1, incremental=True, opt_level=0)
    fixture = H.case('mechanism-diamond', BASE, EXPECTED)
    reference = None
    for name in ('cold', 'warm'):
        for enabled in (False, True):
            mode = 'yes' if enabled else 'no'
            observation = dict(name=name, enabled=enabled, fixture=fixture, config=config)
            evidence['observations'].append(observation)
            profile = root / 'mechanism-profiles' / name / mode
            result = compile_case(rustc, root, fixture, config, enabled, f'mechanism/{mode}', profile=profile)
            observation['result'] = result
            H.validate_expected(result, EXPECTED, name + '/' + mode)
            if reference is None:
                reference = result
            else:
                for component in ('compile', 'runtime'):
                    H.compare(reference[component], result[component], 'mechanism/' + name + '/' + mode)
            observation['profile'] = profile_evidence(profile)
    evidence['proof'] = validate_mechanism(evidence['observations'])


def capture(rustc, root, require_mechanism=False):
    root.mkdir(mode=0o700, parents=False, exist_ok=False)
    report = dict(schema=1, flag=FLAG, scope='correctness-only-no-speedup-claim',
                  diagnostics='exact-bytes-no-normalization', verify_ich=True,
                  configurations=configurations(), planned_cases=len(cases()) * len(configurations()),
                  completed_cases=0, planned_main_compiles=len(cases()) * len(configurations()) * 7,
                  semantic_verdict='incomplete', mechanism_required=require_mechanism,
                  mechanism_verdict='not-requested',
                  verdict='incomplete', observations=[])
    try:
        report['identity_before'] = H.identity(rustc)
        report['feature_probe'] = H.probe(rustc, root)
        if any(record['status'] for record in report['feature_probe'].values()):
            raise AssertionError('Required CGU flag/verify-ICH probe failed, see raw diagnostics')
        for config in configurations():
            for index, fixture in enumerate(cases()):
                observation = dict(config=config, fixture=fixture, modes={})
                report['observations'].append(observation)
                reference = None
                for mode, enabled, cache in schedule(index, config):
                    result = compile_case(rustc, root, fixture, config, enabled, cache)
                    observation['modes'][mode] = result
                    H.validate_expected(result, fixture['expected'], config['name'] + '/' + fixture['name'] + '/' + mode)
                    if reference is None:
                        reference = result
                    else:
                        for component in ('compile', 'runtime'):
                            if component in reference:
                                H.compare(reference[component], result[component], config['name'] + '/' + fixture['name'] + '/' + mode)
                report['completed_cases'] += 1
        report['semantic_verdict'] = 'passed'
        if require_mechanism:
            report['mechanism_verdict'] = 'incomplete'
            report['mechanism_evidence'] = dict(marker_schema=list(MARKERS), profile_events='generic-activity', observations=[])
            mechanism_evidence(rustc, root, report['mechanism_evidence'])
            report['mechanism_verdict'] = 'passed'
        report['verdict'] = 'passed'
    except Exception as error:
        report['verdict'] = 'failed'
        if report['mechanism_verdict'] == 'incomplete':
            report['mechanism_verdict'] = 'failed'
        report['error'] = str(error)
        raise
    finally:
        try:
            report['identity_after'] = H.identity(rustc)
            if report.get('identity_before') != report['identity_after']:
                raise AssertionError('Compiler/driver identity changed during acceptance')
        except Exception as error:
            report['verdict'] = 'failed'
            report['identity_error'] = str(error)
            raise
        finally:
            (root / 'acceptance.json').write_text(json.dumps(report, indent=2) + '\n')
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--rustc', '--compiler', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--require-mechanism', action='store_true')
    args = parser.parse_args()
    os.umask(0o077)
    report = capture(args.rustc.resolve(), args.output.resolve(), args.require_mechanism)
    print(json.dumps({key: report[key] for key in ('verdict', 'semantic_verdict', 'mechanism_verdict', 'completed_cases', 'planned_main_compiles')}))


if __name__ == '__main__':
    try:
        main()
    except Exception as error:
        print(str(error), file=sys.stderr)
        sys.exit(1)
