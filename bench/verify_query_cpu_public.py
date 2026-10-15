#!/usr/bin/env python3
"""Independently replay installed query CPU validation on untouched native captures.

This is an acceptance verifier, not a compiler fixture or performance benchmark.
Mutations are copies outside the accepted cohort. Full public stdout remains in
private evidence files. Only hashes, counts, and validation results are public.
"""
import argparse
from collections import Counter, defaultdict
import copy
import hashlib
import json
from pathlib import Path
import re
import subprocess


def digest(path):
    value = hashlib.sha256()
    with path.open('rb') as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b''):
            value.update(block)
    return value.hexdigest()


def require(condition, message):
    if not condition:
        raise AssertionError(message)


def private_write(path, data):
    with path.open('xb') as stream:
        path.chmod(0o600)
        stream.write(data)


def load_capture(directory):
    documents = [(path, json.loads(path.read_bytes()))
                 for path in sorted(directory.glob('*.json'))]
    queries = [(path, value) for path, value in documents
               if value.get('producer') == 'rustc-query-admission']
    pools = [(path, value) for path, value in documents
             if value.get('producer') == 'rustc-thread-pool-join']
    require(len(queries) == len(pools) == 1, 'one original query and pool required')
    return queries[0], pools[0]


def oracle(query, pool):
    """Derive every qualified owner directly, not from any accepted assessment."""
    require(query['schema'] in (5, 6), 'actual reciprocal query5/6 required')
    require(pool['schema'] == 4 and pool['timing']['schema'] == 2, 'pool4/timing2 required')
    require(query['capture_complete'] and query['lost_events'] == 0,
            'positive replay needs complete original admission capture')
    timing = pool['timing']
    require(timing['records_complete'] and timing['sealed']
            and timing['lost_segments'] == timing['active_segments'] == 0,
            'positive replay needs complete retained segment inventory')
    bridge = timing['clock_bridge']
    require(query['execution_clock'] == bridge, 'original clock must be reciprocal')
    require(bridge['query_capture_id'] == query['capture_id']
            and bridge['pool_capture_id'] == pool['capture_id']
            and bridge['pool_observer'] == pool['observer']
            and bridge['pool_epoch_in_query_ns'] == 0, 'original clock identity mismatch')
    starts, ends = {}, {}
    for index, event in enumerate(query['events']):
        key = event['context_id'], event['job_id']
        target = starts if event['kind'] == 'admitted' else ends
        require(key not in target, 'duplicate original admission lifecycle endpoint')
        target[key] = index, event['at_ns']
    require(starts.keys() == ends.keys(), 'original jobs need both actual endpoints')
    for key, (index, start) in starts.items():
        require(ends[key][0] > index and ends[key][1] >= start, 'invalid actual job lifecycle')
    costs, names = Counter(), {}
    per_query, per_thread = defaultdict(list), defaultdict(list)
    totals = Counter()
    owned_segments = 0
    for row in timing['segments']:
        require(row['status'] == 'measured', 'positive replay requires measured rows')
        start, end = row['start_ns'], row['end_ns']
        require(start <= end and row['cpu_end_ns'] - row['cpu_start_ns'] == row['cpu_ns']
                and row['cpu_ns'] <= end - start, 'original CPU/wall delta mismatch')
        per_thread[row['thread_id']].append((start, end))
        totals[row['owner_kind']] += row['cpu_ns']
        context, job, name = row['query_context'], row['query_job'], row['query_name']
        if row['owner_kind'] != 'query_execution':
            require(context is job is name is None, 'nonquery segment carried query labels')
            continue
        require(type(context) is int and context > 0 and type(job) is int and job > 0,
                'query owner must have qualified positive endpoints')
        require(type(name) is str and re.fullmatch(r'[A-Za-z_][A-Za-z0-9_]{0,127}', name),
                'query category is not a bounded static identifier')
        key = context, job
        require(key in starts, 'CPU owner is not actually admitted in this capture')
        require(starts[key][1] <= start <= end <= ends[key][1], 'CPU outside actual query lifetime')
        require(names.setdefault(key, name) == name, 'query category changed within one job')
        costs[key] += row['cpu_ns']
        per_query[key].append((start, end))
        owned_segments += 1
    for intervals in [*per_query.values(), *per_thread.values()]:
        intervals.sort()
        require(all(left[1] <= right[0] for left, right in zip(intervals, intervals[1:])),
                'original exclusive CPU intervals overlap on one owner/thread')
    ranked = [dict(query_context=context, query_job=job, query_name=names[context, job], cpu_ns=cpu)
              for (context, job), cpu in costs.items()]
    ranked.sort(key=lambda row: (-row['cpu_ns'], row['query_context'], row['query_job']))
    require(owned_segments > 0 and sum(costs.values()) > 0, 'positive replay cannot be all unknown')
    return ranked, owned_segments, totals


def assess(value, expected):
    for row in value:
        require(row['full_pipeline_complete'] is False and row['cpu_work_ns'] is None
                and row['execution_span_ns'] is None, 'public replay promoted full work/span')
    query = next(row for row in value if row['producer'] == 'rustc-query-admission')
    pool = next(row for row in value if row['producer'] == 'rustc-thread-pool-join')
    ranked, segments, totals = expected
    ownership, cpu = query['query_cpu_ownership'], pool['observed_cpu']
    require(ownership['scope'] == 'observed-query-execution-cpu', 'wrong public CPU scope')
    require(ownership['records_complete'] and cpu['records_complete'], 'positive ownership unknown')
    require(ownership['queries'] == ranked and ownership['measured_segments'] == segments,
            'public owner ranking does not match every original segment')
    require(ownership['query_execution_cpu_ns'] == totals['query_execution']
            == cpu['query_execution_cpu_ns'], 'public query CPU sum mismatch')
    require(cpu['task_cpu_ns'] == totals['task'] and cpu['opaque_cpu_ns'] == totals['opaque']
            and cpu['observed_cpu_ns'] == sum(totals.values()), 'exclusive owner partition mismatch')
    require(cpu['query_clock_aligned'] and query['clock_alignment']['aligned'],
            'positive replay lacks reciprocal clock')
    return dict(query_schema=query['schema'], measured_query_segments=segments,
                observed_query_owners=len(ranked), query_execution_cpu_ns=totals['query_execution'],
                task_cpu_ns=totals['task'], opaque_cpu_ns=totals['opaque'],
                observed_cpu_ns=sum(totals.values()), every_original_owner_checked=True)


def main():
    parser = argparse.ArgumentParser(__doc__)
    parser.add_argument('--justrust', type=Path, required=True)
    parser.add_argument('--accepted', type=Path, required=True)
    parser.add_argument('--evidence', type=Path, required=True)
    args = parser.parse_args()
    accepted, evidence = args.accepted.resolve(), args.evidence.resolve()
    require(not evidence.is_relative_to(accepted), 'evidence must be outside accepted cohort')
    marker = accepted / 'query-cpu-acceptance.json'
    require(marker.is_file(), 'wait for completed native query-cpu-acceptance.json')
    evidence.mkdir(mode=0o700, parents=True, exist_ok=False)
    labels = ['query-cpu-cold-1', 'query-cpu-warm-1', 'query-cpu-with-dispatch']
    captures = {label: accepted / (label + '-capture') for label in labels}
    paths = [marker, accepted / 'provenance.json']
    paths.extend(path for directory in captures.values() for path in sorted(directory.glob('*.json')))
    before = {str(path.relative_to(accepted)): digest(path) for path in paths}
    consumer_hash = digest(args.justrust)
    cases = []

    def run(label, path, flags, failure=None):
        command = [str(args.justrust), 'graph', str(path), '--json', *flags]
        completed = subprocess.run(command, capture_output=True, check=False)
        private_write(evidence / (label + '.stdout'), completed.stdout)
        private_write(evidence / (label + '.stderr'), completed.stderr)
        record = dict(case=label, flags=flags, exit=completed.returncode,
                      stdout_sha256=hashlib.sha256(completed.stdout).hexdigest(),
                      stdout_bytes=len(completed.stdout))
        cases.append(record)
        if failure:
            require(completed.returncode == 1 and failure in completed.stderr.decode(),
                    f'{label}: expected strict public refusal: {completed.stderr.decode()[:1000]}')
            record['check'] = 'public refusal: ' + failure
            return None, record
        require(completed.returncode == 0,
                f'{label}: public CLI failed: {completed.stderr.decode()[:1000]}')
        return json.loads(completed.stdout), record

    cpu_flags = ['--large-capture', '--query-cpu-ownership']
    for label, directory in captures.items():
        (query_path, query), (pool_path, pool) = load_capture(directory)
        expected = oracle(query, pool)
        value, record = run(label + '-cpu-only', directory, cpu_flags)
        record['checks'] = assess(value, expected)
        public_query = next(row for row in value if row['producer'] == 'rustc-query-admission')
        require('dispatch_result_provenance' not in public_query
                and 'cache_call_provenance' not in public_query,
                'CPU-only implicitly enabled availability analysis')
        del value
        run(label + '-default-pool4-refusal', pool_path, [], failure='unsupported join protocol')
        if query['schema'] == 6:
            value, record = run(label + '-cpu-dispatch', directory,
                                cpu_flags + ['--dispatch-result-provenance'])
            record['checks'] = assess(value, expected)
            q = next(row for row in value if row['producer'] == 'rustc-query-admission')
            require(q['query_cpu_ownership'] == public_query['query_cpu_ownership']
                    and q['dispatch_result_provenance']['resolved_requests'] > 0,
                    'combined dispatch changed independent CPU ownership or resolved no actual dispatch')
            record['dispatch_resolved_requests'] = q['dispatch_result_provenance']['resolved_requests']
            del value, q
            extra = ['--dispatch-result-provenance', '--cache-call-provenance']
        else:
            extra = ['--cache-call-provenance']
        value, record = run(label + '-combined', directory, cpu_flags + extra)
        record['checks'] = assess(value, expected)
        q = next(row for row in value if row['producer'] == 'rustc-query-admission')
        require(q['query_cpu_ownership'] == public_query['query_cpu_ownership'],
                'availability opt-ins changed independent ownership')
        require(q['cache_call_provenance']['records_complete'], 'combined cache inventory incomplete')
        del value, q, query, pool, public_query, expected

    # Negative variants are copied only from the small real warm cohort.
    (query_path, query), (pool_path, pool) = load_capture(captures['query-cpu-warm-1'])
    for variant in ['missing-query', 'foreign-context', 'clock-mismatch']:
        directory = evidence / variant
        directory.mkdir(mode=0o700)
        changed_pool = copy.deepcopy(pool)
        if variant != 'missing-query':
            changed_query = copy.deepcopy(query)
            if variant == 'clock-mismatch':
                changed_query['execution_clock']['pool_observer'] += 1
            private_write(directory / query_path.name, json.dumps(changed_query).encode())
            del changed_query
        if variant == 'foreign-context':
            row = next(row for row in changed_pool['timing']['segments']
                       if row['owner_kind'] == 'query_execution')
            row['query_context'] = max(context['context_id'] for context in query['contexts']) + 1
        private_write(directory / pool_path.name, json.dumps(changed_pool).encode())
        value, record = run(variant, directory, cpu_flags,
                            failure='not an actual same-capture admitted job'
                            if variant == 'foreign-context' else None)
        if value is not None:
            for row in value:
                require(row['cpu_work_ns'] is None and row['execution_span_ns'] is None
                        and row['full_pipeline_complete'] is False, 'negative case promoted work/span')
                if row['producer'] == 'rustc-query-admission':
                    report = row['query_cpu_ownership']
                    require(report['query_execution_cpu_ns'] is None
                            and not report['records_complete'] and report['queries'] == [],
                            'unqualified query authority acquired a public ranking')
                if row['producer'] == 'rustc-thread-pool-join':
                    cpu = row['observed_cpu']
                    require(cpu['query_execution_cpu_ns'] is None and not cpu['records_complete']
                            and not cpu['query_clock_aligned'], 'unqualified pool acquired query CPU')
            record['check'] = 'unknown ownership, no rankings, no scoped/full completeness promotion'
        del value, changed_pool

    after = {str(path.relative_to(accepted)): digest(path) for path in paths}
    require(before == after, 'original native inputs changed during public replay')
    require(digest(args.justrust) == consumer_hash, 'installed consumer changed during replay')
    summary = dict(schema=1, scope='installed-public-query-cpu-original-native-acceptance-not-a-benchmark',
                   consumer_sha256=consumer_hash, cases=cases,
                   unchanged_original_input_files=len(before),
                   original_input_inventory_sha256=hashlib.sha256(
                       json.dumps(before, sort_keys=True).encode()).hexdigest(),
                   original_inputs=[dict(name=name, bytes=(accepted / name).stat().st_size, sha256=sha)
                                    for name, sha in sorted(before.items())],
                   all_checks_passed=True,
                   limitations=['Replays actual native receipts, not a compiler rerun or speed benchmark.',
                                'Names identify bounded static query categories, not keys or provider freshness.',
                                'Observed exclusive CPU sums are not full work, a span, an execution DAG or a thread ceiling.',
                                'Compilation/artifact neutrality and analyzer acceptance remain in the native cohort.'])
    private_write(evidence / 'summary.json', (json.dumps(summary, indent=2) + '\n').encode())
    print(json.dumps(summary, indent=2))


if __name__ == '__main__':
    main()
