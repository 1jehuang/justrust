#!/usr/bin/env python3
"""Frozen-compiler demand-cache correctness gate, never a speed benchmark."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import sys


def load(name, filename):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(filename))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


H = load('demand_common', 'reuse-declarative-macro-accept.py')
SCALAR = load('demand_scalar', 'reuse-query-accept.py')
FLAG = '-Zjr-demand-query-cache'
H.FLAG = FLAG
PROMOTION = 'incr_comp_query_cache_promotion'
SKIPPED = 'jr_demand_query_cache_promotion_skipped'
GREEN = 'jr_incremental_green_recompute'
MARKERS = (PROMOTION, SKIPPED, GREEN)
DEMAND = '''#![allow(dead_code)]
trait Number { fn number() -> u32; }
impl Number for u32 { fn number() -> u32 { 7 } }
impl Number for u64 { fn number() -> u32 { 11 } }
fn selected<T: Number>() -> u32 { T::number() }
fn body() -> u32 { 1 }
fn main() { println!("{}", selected::<u32>() + selected::<u64>() + body()); }
'''
NARROW = DEMAND.replace('selected::<u32>() + selected::<u64>()', 'selected::<u32>()')


def cases():
    scalar = [H.case(name, source.replace('retention_dep', 'macro_dep'), expected,
                     dependency=dep) for name, source, dep, expected in SCALAR.cases()]
    return scalar + [
        H.case('demand-wide', DEMAND, '19\n'),
        H.case('demand-wide-unchanged', DEMAND, '19\n'),
        H.case('demand-narrow', NARROW, '8\n'),
        H.case('demand-narrow-unchanged', NARROW, '8\n'),
        H.case('demand-narrow-body-edit', NARROW.replace('{ 1 }', '{ 2 }'), '9\n'),
        H.case('demand-wide-return', DEMAND, '19\n'),
        H.case('demand-wide-return-unchanged', DEMAND, '19\n'),
        H.case('demand-error', NARROW.replace('{ 1 }', '{ "wrong" }'), None),
        H.case('demand-error-recovery', DEMAND, '19\n'),
        H.case('demand-recovery-unchanged', DEMAND, '19\n'),
    ]


def schedule(index):
    return [('fresh-no', False, f'fresh/{index}/no'),
            ('fresh-yes', True, f'fresh/{index}/yes'),
            ('persistent-no', False, 'persistent/no'),
            ('persistent-no-replay', False, 'persistent/no'),
            ('persistent-yes', True, 'persistent/yes'),
            ('persistent-yes-replay', True, 'persistent/yes'),
            ('transition-no-before', False, 'transition'),
            ('transition-yes', True, 'transition'),
            ('transition-no-after', False, 'transition')]


def profile_evidence(directory):
    paths = sorted(directory.glob('*.mm_profdata'))
    if len(paths) != 1:
        raise AssertionError(f'Expected exactly one native profile in {directory}')
    q = load('demand_query_profile', 'query_profile.py')
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
                raise AssertionError('Unexpected native marker kind/type')
            counts[event['label']] += 1
    return dict(path=str(paths[0]), sha256=H.digest(paths[0]), bytes=len(raw),
                markers=counts, records=records, unresolved_records=unresolved,
                metadata=q.metadata_from_strings(strings))


def validate_mechanism(observations):
    expected = [(name, enabled) for name in
                ('cold', 'unchanged', 'demand-narrow', 'demand-narrow-unchanged', 'demand-wide-return')
                for enabled in (False, True)]
    if [(o['name'], o['enabled']) for o in observations] != expected:
        raise AssertionError('Incomplete or reordered mechanism evidence sequence')
    for observation in observations:
        counts = observation['profile']['markers']
        if observation['enabled']:
            if not (counts[SKIPPED] > 0 and counts[PROMOTION] == 0):
                raise AssertionError('On path lacks positive promotion-skipped evidence or ran normal promotion')
        elif not (counts[PROMOTION] > 0 and counts[SKIPPED] == 0):
            raise AssertionError('Off control lacks normal promotion or emitted promotion-skipped marker')
    changed = [o for o in observations if o['enabled'] and o['name'] == 'demand-wide-return']
    return dict(proof='positive skipped promotion versus positive normal-promotion controls',
                green_recompute_after_demand_return=sum(o['profile']['markers'][GREEN] for o in changed),
                limitation='Green recompute markers are observational, not proof that a particular missing value caused fallback. No speedup claim.')


def mechanism_evidence(rustc, root, evidence):
    fixtures = [H.case('cold', DEMAND, '19\n'),
                H.case('unchanged', DEMAND, '19\n'),
                H.case('demand-narrow', NARROW, '8\n'),
                H.case('demand-narrow-unchanged', NARROW, '8\n'),
                H.case('demand-wide-return', DEMAND, '19\n')]
    for fixture in fixtures:
        reference = None
        for enabled in (False, True):
            mode = 'yes' if enabled else 'no'
            observation = dict(name=fixture['name'], enabled=enabled, fixture=fixture)
            evidence['observations'].append(observation)
            directory = root / 'mechanism-profiles' / fixture['name'] / mode
            result = H.compile_case(rustc, root, fixture, enabled, f'mechanism/{mode}', profile=directory)
            observation['result'] = result
            H.validate_expected(result, fixture['expected'], fixture['name'])
            if reference is not None:
                for component in ('compile', 'runtime'):
                    H.compare(reference[component], result[component], fixture['name'])
            reference = result
            observation['profile'] = profile_evidence(directory)
    evidence['proof'] = validate_mechanism(evidence['observations'])


def capture(rustc, root, require_mechanism=False):
    root.mkdir(mode=0o700, parents=False, exist_ok=False)
    report = dict(schema=1, flag=FLAG, scope='correctness-only-no-speedup-claim',
                  verify_ich=True, diagnostics='exact-bytes-no-normalization',
                  planned_cases=len(cases()), completed_cases=0, observations=[],
                  semantic_verdict='incomplete', mechanism_required=require_mechanism,
                  mechanism_verdict='not-requested', verdict='incomplete')
    try:
        report['identity_before'] = H.identity(rustc)
        report['feature_probe'] = H.probe(rustc, root)
        if any(r['status'] for r in report['feature_probe'].values()):
            raise AssertionError('Required demand-cache yes/no or verify-ich probe failed, see raw diagnostics')
        for index, fixture in enumerate(cases()):
            observation = dict(fixture=fixture, modes={})
            report['observations'].append(observation)
            reference = None
            for name, enabled, cache in schedule(index):
                result = H.compile_case(rustc, root, fixture, enabled, cache)
                observation['modes'][name] = result
                H.validate_expected(result, fixture['expected'], fixture['name'] + '/' + name)
                if reference is None:
                    reference = result
                else:
                    for component in ('dependency', 'compile', 'runtime'):
                        if component in reference:
                            H.compare(reference[component], result[component], fixture['name'] + '/' + name)
            report['completed_cases'] += 1
        report['semantic_verdict'] = 'passed'
        if require_mechanism:
            report['mechanism_verdict'] = 'incomplete'
            report['mechanism_evidence'] = dict(marker_schema=list(MARKERS), observations=[])
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
    parser.add_argument('--require-mechanism', action='store_true',
                        help='Separately require profiled positive skipped/normal promotion evidence')
    args = parser.parse_args()
    os.umask(0o077)
    report = capture(args.rustc.resolve(), args.output.resolve(), args.require_mechanism)
    print(json.dumps({k: report[k] for k in ('verdict', 'semantic_verdict', 'mechanism_verdict', 'completed_cases')}))


if __name__ == '__main__':
    try:
        main()
    except Exception as error:
        print(str(error), file=sys.stderr)
        sys.exit(1)
