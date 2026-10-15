#!/usr/bin/env python3
"""Replay the installed public CLI against retained, untouched real Rust builds.

This is an acceptance verifier, not a benchmark or a synthetic compiler fixture.
Full CLI stdout stays in a private evidence directory. The emitted compact
summary records checks and hashes without publishing private paths.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess


def sha(path):
    h = hashlib.sha256()
    with path.open('rb') as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b''):
            h.update(block)
    return h.hexdigest()


def inventory(paths):
    return {str(p): sha(p) for directory in paths for p in sorted(directory.glob('*.json'))}


def main():
    ap = argparse.ArgumentParser(__doc__)
    ap.add_argument('--justrust', type=Path, required=True)
    ap.add_argument('--accepted', type=Path, required=True)
    ap.add_argument('--evidence', type=Path, required=True)
    args = ap.parse_args()
    args.evidence.mkdir(mode=0o700, parents=True, exist_ok=False)
    accepted = args.accepted
    cold = accepted / 'dispatch-cold-t4-1-capture'
    warm = accepted / 'dispatch-warm-t4-1-capture'
    mixed = accepted / 'dispatch-with-timing-capture'
    replay4 = json.loads((accepted / 'dispatch-query4-replay.json').read_text())
    replay5 = json.loads((accepted / 'dispatch-query5-replay.json').read_text())
    legacy4 = Path(replay4['command'][2])
    legacy5 = Path(replay5['command'][2])
    before = inventory([cold, warm, mixed, legacy4, legacy5])
    consumer_before = sha(args.justrust)
    results = []

    def run(label, path, flags, expected_exit=0, expected=None):
        argv = [str(args.justrust), 'graph', str(path), '--json', *flags]
        completed = subprocess.run(argv, stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=False)
        for suffix, data in [('stdout', completed.stdout), ('stderr', completed.stderr)]:
            target = args.evidence / f'{label}.{suffix}'
            with target.open('xb') as stream:
                target.chmod(0o600)
                stream.write(data)
        assert completed.returncode == expected_exit, (label, completed.returncode, completed.stderr.decode()[:1000])
        record = {'case': label, 'flags': flags, 'exit': completed.returncode,
                  'stdout_sha256': hashlib.sha256(completed.stdout).hexdigest(),
                  'stdout_bytes': len(completed.stdout), 'checks': []}
        if expected is not None:
            assert completed.stdout.decode() == expected, f'{label}: legacy bytes changed'
            record['checks'].append('exact frozen legacy stdout bytes')
        if expected_exit != 0:
            record['checks'].append('public CLI refused unsupported/default analysis')
            record['stderr'] = completed.stderr.decode().strip()
            results.append(record)
            return None
        value = json.loads(completed.stdout)
        for assessment in value:
            assert assessment.get('full_pipeline_complete') is False
            assert assessment.get('cpu_work_ns') is None
            assert assessment.get('execution_span_ns') is None
        record['checks'].append('full pipeline remains incomplete and CPU work/span unknown')
        results.append(record)
        return value

    run('root6-default-refusal', warm, [], expected_exit=1)
    assert 'unsupported query receipt schema' in results[-1]['stderr']
    run('cold-strict-budget-refusal', cold, ['--dispatch-result-provenance'], expected_exit=1)
    assert 'byte budget' in results[-1]['stderr']
    cold_value = run('cold-both', cold, ['--large-capture', '--dispatch-result-provenance', '--cache-call-provenance'])
    cold_query = next(a for a in cold_value if a.get('dispatch_result_provenance') is not None)
    dispatch = cold_query['dispatch_result_provenance']
    cache = cold_query['cache_call_provenance']
    assert dispatch['observed_dispatch_resolutions'] == 18297
    assert dispatch['resolved_requests'] == len(dispatch['relations']) == 17669
    assert cache['resolved_requests'] == len(cache['relations']) == 153723
    assert cold_query['query_reads']['semantic_complete'] is False
    assert all(a.get('resource_attachments', {}).get('state') != 'validated' for a in cold_value)
    results[-1]['checks'] += ['18297 actual dispatch indices produce 17669 certified availability relations',
                              '153723 independent cache relations',
                              'semantic and resource authority are not promoted']
    results[-1]['dispatch_resolved'] = dispatch['resolved_requests']
    results[-1]['cache_resolved'] = cache['resolved_requests']
    del cold_value, cold_query, dispatch, cache

    warm_dispatch = run('warm-dispatch-only', warm, ['--dispatch-result-provenance'])
    warm_both = run('warm-both', warm, ['--dispatch-result-provenance', '--cache-call-provenance'])
    query = next(a for a in warm_both if a.get('dispatch_result_provenance') is not None)
    assert query['dispatch_result_provenance']['observed_dispatch_resolutions'] == 0
    assert query['dispatch_result_provenance']['resolved_requests'] == 0
    assert query['cache_call_provenance']['resolved_requests'] == 3580
    for assessment in warm_both:
        assessment.pop('cache_call_provenance', None)
    assert warm_dispatch == warm_both, 'cache opt-in altered independent public output'
    results[-1]['checks'] += ['incremental dispatch stays explicitly unobserved', '3580 cache relations remain separate',
                              'removing optional cache field exactly restores dispatch-only public output']
    del warm_dispatch, warm_both, query

    mixed_value = run('mixed-timing', mixed, ['--large-capture', '--dispatch-result-provenance', '--cache-call-provenance'])
    query = next(a for a in mixed_value if a.get('dispatch_result_provenance') is not None)
    assert query['clock_alignment']['aligned'] is True
    cpu = next(a['observed_cpu'] for a in mixed_value if a.get('observed_cpu') is not None)
    assert cpu['records_complete'] is True
    assert query['dispatch_result_provenance']['resolved_requests'] == 17603
    results[-1]['checks'] += ['reciprocal timing clock aligned', 'complete retained exclusive CPU segments coexist with 17603 dispatch relations']
    del mixed_value, query, cpu

    run('legacy-root4', legacy4, ['--large-capture'], expected=replay4['stdout'])
    run('legacy-root5', legacy5, ['--large-capture'], expected=replay5['stdout'])
    after = inventory([cold, warm, mixed, legacy4, legacy5])
    assert before == after, 'original accepted capture input changed'
    assert sha(args.justrust) == consumer_before, 'installed consumer changed during acceptance'
    summary = {'schema': 1, 'scope': 'installed-public-cli-retained-real-source-acceptance-not-a-benchmark',
               'consumer_sha256': consumer_before, 'cases': results,
               'original_inputs': [{'name': Path(p).name, 'bytes': Path(p).stat().st_size, 'sha256': h}
                                   for p, h in sorted(before.items())],
               'unchanged_original_input_files': len(before),
               'original_input_inventory_sha256': hashlib.sha256(json.dumps(before, sort_keys=True).encode()).hexdigest(),
               'all_checks_passed': True,
               'limitations': ['Replays actual untouched real-build receipts, does not rerun compilation.',
                               'Artifact neutrality and lexical row oracles are recorded in the original real-source acceptance.',
                               'Availability relations are not a complete execution DAG or a thread ceiling.',
                               'Comprehensive performance remains blocked by the separately audited baseline.']}
    (args.evidence / 'summary.json').write_text(json.dumps(summary, indent=2) + '\n')
    print(json.dumps(summary, indent=2))


if __name__ == '__main__':
    main()
