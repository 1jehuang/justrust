#!/usr/bin/env python3
"""Replay completed native latch-readiness cohorts without changing original bytes.

This is independent public acceptance, not a benchmark or a full critical path.
Full stdout, copied controls and before/after inventories remain private.
"""
import argparse
from collections import Counter
import copy
import hashlib
import json
from pathlib import Path
import subprocess
import sys

from latch_readiness_oracle import latch_readiness_oracle
from execution_flow_oracle import _known_chain
from verify_execution_flow_public import witness, check_compact
from verify_query_cpu_public import digest, load_capture, private_write, require

from verify_execution_flow_public import COUNTS as FLOW_COUNTS, COSTS

READY_COUNTS = ('readiness_rows', 'active_readiness', 'lost_readiness', 'wait_attempts',
                'active_wait_attempts', 'lost_wait_attempts', 'matched_readiness_transfers',
                'unbound_wait_attempts', 'unavailable_readiness_references')
COUNTS = (*FLOW_COUNTS, *READY_COUNTS)
LABELS = ('readiness-cold-1', 'readiness-warm-1', 'readiness-with-dispatch')


def scoped(report):
    require(report['scope'] == 'observed-query-join-and-latch-control-flow', 'incorrect readiness scope')
    require(report['full_pipeline_complete'] is False and report['flow_complete'] is False,
            'readiness promoted completeness beyond observed instrumentation')
    require(all(report[key] is None for key in ('cpu_work_ns', 'execution_span_ns', 'parallelism')),
            'readiness promoted full work, span or parallelism')


def check_flow(report, expected, pool):
    scoped(report)
    for key, value in expected.items():
        if key not in ('longest_known_chain_path', 'longest_known_chain_steps'):
            require(report[key] == value, 'original readiness oracle mismatch: ' + key)
    require(isinstance(report['issues'], list), 'missing readiness issues')
    if report['records_complete']:
        require(report['issues'] == [], 'complete readiness acquired issues')
    else:
        require(report['issues'], 'partial readiness must explain unavailable authority')
    require(isinstance(report['limitations'], list) and report['limitations'],
            'instrumentation limitations must remain separate from issues')
    witness(report, pool)
    return dict(**{key: report[key] for key in (*COUNTS, *COSTS)},
                every_original_count_cost_and_witness_checked=True)


def check_unqualified(report, query, pool):
    # No positive oracle is used to authorize missing or mismatched query sources.
    # Independent nonquery measurements remain original facts, not query authority.
    original = latch_readiness_oracle(query, pool, allow_unbound=True)
    for key in COUNTS:
        if key not in ('unavailable_cpu_segments', 'matched_readiness_transfers',
                       'unavailable_readiness_references'):
            require(report[key] == original[key], 'unqualified original inventory changed: ' + key)
    segments = {row['segment_id']: row for row in pool['timing']['segments']}
    nodes = {row['id']: row for row in pool['execution_flow']['nodes']}
    edges = {(row['from'], row['to']) for row in pool['execution_flow']['edges']}
    costs = {}
    for ident, node in nodes.items():
        segment = segments.get(node['segment_id'])
        if segment:
            costs[ident] = None if segment['owner_kind'] == 'query_execution' else segment['cpu_ns']
        else:
            costs[ident] = None if node['kind'] in ('cut', 'wait_consumed') else 0
    maximum, _ = _known_chain(nodes, edges, costs)
    require(report['longest_known_chain_cpu_ns'] == maximum,
            'unqualified witness is not the independent maximum retained-known chain')
    require(all(costs[ident] is not None for ident in report['longest_known_chain_path']),
            'unqualified witness crossed an unknown readiness or CPU barrier')
    totals, counts = Counter(), Counter()
    for node in pool['execution_flow']['nodes']:
        segment = segments.get(node['segment_id'])
        if segment and segment['owner_kind'] != 'query_execution':
            totals[segment['owner_kind']] += segment['cpu_ns']
            counts[segment['owner_kind']] += 1
    require(report['known_query_cpu_ns'] is None, 'missing query authority retained query CPU')
    for key in ('nodes', 'edges', 'scopes'):
        require(report[key] == len(pool['execution_flow'][key]),
                'missing authority changed original flow inventory: ' + key)
    require(report['unavailable_cpu_segments'] == sum(
        segments[node['segment_id']]['owner_kind'] == 'query_execution'
        for node in pool['execution_flow']['nodes'] if node['segment_id'] in segments),
        'missing authority did not withdraw every query-owned CPU segment')
    for category in ('task', 'opaque'):
        require(report['known_' + category + '_cpu_ns'] ==
                (totals[category] if counts[category] else None), 'independent CPU mass changed')
    require(report['known_cpu_work_ns'] == (sum(totals.values()) if sum(counts.values()) else None),
            'unqualified known CPU work changed')
    require(report['matched_readiness_transfers'] == 0, 'missing authority fabricated readiness transfer')
    require(report['ready_records_complete'] is False and report['wait_records_complete'] is False,
            'missing authority fabricated complete readiness inventories')
    witness(report, pool, query_authorized=False)


def check_report_inventory(actual, expected):
    """Compare every report field and exact identity cardinality, not row order."""
    def inventory(rows):
        identities = [(row['producer'], row['capture_id']) for row in rows]
        require(len(set(identities)) == len(identities), 'duplicate public report identity')
        return len(rows), Counter(json.dumps(row, sort_keys=True) for row in rows)
    require(inventory(actual) == inventory(expected), 'public report inventory or fields changed')


def test_report_inventory():
    """Offline regression checks only, never produces native acceptance evidence."""
    first = dict(producer='query', capture_id='q', fact=7, resource_attachments={'state': 'unknown'})
    second = dict(producer='pool', capture_id='p', fact=9)
    original = [first, second]
    check_report_inventory(list(reversed(original)), original)
    for changed in ([dict(first, fact=8), second], [first, second, second],
                    [first], [dict(first, resource_attachments={'state': 'fabricated'}), second]):
        try:
            check_report_inventory(changed, original)
        except AssertionError:
            continue
        raise AssertionError('report inventory mutation accepted')


def neutral_snapshot(directory, reference):
    """Copy exact original bytes in analyzer scandir-to-neutral-name order."""
    reference.mkdir(mode=0o700)
    sources = [path for path in directory.iterdir() if path.suffix == '.json']
    require(len(sources) == 2, 'neutral reference requires original query/pool pair only')
    mapping = []
    taken = {source.name for source in sources}
    next_name = 0
    for source in sources:
        while True:
            name = 'frontend-query-' + str(next_name) + '.json'
            next_name += 1
            if name not in taken:
                break
        taken.add(name)
        raw = source.read_bytes()
        private_write(reference / name, raw)
        mapping.append(dict(source=source.name, neutral=name, bytes=len(raw),
                            sha256=hashlib.sha256(raw).hexdigest()))
    check_neutral_snapshot(directory, reference, mapping)
    return mapping


def check_neutral_snapshot(directory, reference, mapping):
    require(sorted(path.name for path in reference.iterdir()) ==
            sorted(row['neutral'] for row in mapping), 'neutral snapshot inventory changed')
    for row in mapping:
        source, target = directory / row['source'], reference / row['neutral']
        require(source.stat().st_size == target.stat().st_size == row['bytes']
                and digest(source) == digest(target) == row['sha256'],
                'neutral snapshot changed exact original source bytes')
        require(target.stat().st_mode & 0o777 == 0o600, 'neutral snapshot file is not private')


def test_neutral_snapshot():
    """Offline routing and byte-preservation regression, not capture acceptance."""
    import tempfile
    with tempfile.TemporaryDirectory(prefix='jr-readiness-verifier-test-') as temporary:
        root = Path(temporary)
        sources = root / 'sources'
        sources.mkdir()
        private_write(sources / 'z-query.json', b'{ "original": "query" }\n')
        private_write(sources / 'a-pool.json', b'{ "original": "pool" }\n')
        before = [(path.name, path.read_bytes()) for path in sources.iterdir()]
        reference = root / 'reference'
        mapping = neutral_snapshot(sources, reference)
        require([(row['source'], row['neutral']) for row in mapping] ==
                [(name, 'frontend-query-' + str(index) + '.json')
                 for index, (name, _) in enumerate(before)], 'source-to-neutral map changed')
        require(sorted(path.name for path in reference.iterdir()) ==
                ['frontend-query-0.json', 'frontend-query-1.json'], 'incorrect neutral names')
        for index, (_, raw) in enumerate(before):
            require((reference / ('frontend-query-' + str(index) + '.json')).read_bytes() == raw,
                    'neutral routing changed source order or original bytes')
        require(before == [(path.name, path.read_bytes()) for path in sources.iterdir()],
                'snapshot modified source inventory')
        (reference / 'frontend-query-0.json').write_bytes(b'altered')
        try:
            check_neutral_snapshot(sources, reference, mapping)
        except AssertionError:
            pass
        else:
            raise AssertionError('altered neutral snapshot accepted')
        collision = root / 'collision'
        collision.mkdir()
        private_write(collision / 'frontend-query-0.json', b'first-original')
        private_write(collision / 'other.json', b'second-original')
        collision_mapping = neutral_snapshot(collision, root / 'collision-reference')
        require([row['neutral'] for row in collision_mapping] ==
                ['frontend-query-1.json', 'frontend-query-2.json'],
                'neutral allocator overwrote an original source name')


def main():
    parser = argparse.ArgumentParser(__doc__)
    parser.add_argument('--accepted', type=Path, required=True)
    parser.add_argument('--justrust', type=Path, required=True)
    parser.add_argument('--evidence', type=Path, required=True)
    args = parser.parse_args()
    accepted, evidence, consumer = args.accepted.resolve(), args.evidence.resolve(), args.justrust.resolve()
    require(not evidence.is_relative_to(accepted), 'evidence must be outside original accepted cohort')
    require((accepted / 'latch-readiness-acceptance.json').is_file(),
            'wait for completed native latch-readiness-acceptance.json')
    marker = json.loads((accepted / 'latch-readiness-acceptance.json').read_bytes())
    require([row['label'] for row in marker['observations']] == list(LABELS),
            'completed marker does not identify the expected original cohorts')
    require(marker.get('allow_conservative_unbound') is True,
            'completed marker must explicitly admit conservative original unbound waits')
    evidence.mkdir(mode=0o700, parents=True, exist_ok=False)
    require(evidence.stat().st_mode & 0o777 == 0o700, 'evidence directory is not private')

    def inventory():
        return {str(path.relative_to(accepted)): dict(bytes=path.stat().st_size, sha256=digest(path))
                for path in sorted(accepted.rglob('*')) if path.is_file()}

    before = inventory()
    binary_before = dict(bytes=consumer.stat().st_size, sha256=digest(consumer))
    private_write(evidence / 'inputs-before.json', json.dumps(before, sort_keys=True).encode())
    private_write(evidence / 'binary-before.json', json.dumps(binary_before, sort_keys=True).encode())
    cases = []

    def execute(label, command, refusal=False, parse_json=True):
        result = subprocess.run(command, capture_output=True, timeout=300)
        private_write(evidence / (label + '.stdout'), result.stdout)
        private_write(evidence / (label + '.stderr'), result.stderr)
        record = dict(case=label, exit=result.returncode,
                      stdout_sha256=hashlib.sha256(result.stdout).hexdigest(), stdout_bytes=len(result.stdout))
        cases.append(record)
        if refusal:
            require(result.returncode != 0, label + ': required public refusal succeeded')
            record['public_refusal_checked'] = True
            return None, record
        require(result.returncode == 0, label + ': public replay failed: ' + result.stderr.decode(errors='replace'))
        return (json.loads(result.stdout) if parse_json else None), record

    def graph(label, directory, flags=(), refusal=False):
        return execute(label, [str(consumer), 'graph', str(directory), '--json', '--large-capture', *flags], refusal)

    def pool_flow(values):
        rows = [row for row in values if row['producer'] == 'rustc-thread-pool-join']
        require(len(rows) == 1, 'public replay requires one original pool assessment')
        return rows[0]['latch_readiness_dag']

    def no_full(values):
        for row in values:
            require(row['full_pipeline_complete'] is False and row['cpu_work_ns'] is None
                    and row['execution_span_ns'] is None, 'public replay promoted full work/span')

    try:
        warm = None
        for label in LABELS:
            directory = accepted / (label + '-capture')
            (query_path, query), (pool_path, pool) = load_capture(directory)
            require(query['schema'] == (6 if label == 'readiness-with-dispatch' else 5),
                    'actual cohort query schema mismatch')
            require(pool['schema'] == 6, 'readiness cohort requires original pool6')
            require(marker['original_sha256'][label] ==
                    {path.name: digest(path) for path in directory.glob('*.json')},
                    'original capture bytes differ from completed acceptance marker')
            expected = latch_readiness_oracle(query_path, pool_path, allow_unbound=True)
            values, record = graph(label + '-flow-only', directory, ['--latch-readiness-dag'])
            no_full(values)
            public = pool_flow(values)
            record['checks'] = check_flow(public, expected, pool)
            for row in values:
                require(all(row.get(key) is None for key in
                            ('query_cpu_ownership', 'cache_call_provenance', 'dispatch_result_provenance', 'observed_task_dag', 'execution_flow_dag')),
                        'flow-only implied unrelated public diagnostics')
            flags = ['--latch-readiness-dag', '--query-cpu-ownership', '--execution-flow-dag']
            if query['schema'] == 6:
                flags.append('--dispatch-result-provenance')
            combined, record = graph(label + '-combined', directory, flags)
            no_full(combined)
            require(pool_flow(combined) == public, 'CPU/dispatch opt-ins changed independent flow assessment')
            record['independent_flag_equality_checked'] = True
            graph(label + '-default-pool6-refusal', pool_path, refusal=True)
            graph(label + '-old-cpu-pool6-refusal', pool_path, ['--query-cpu-ownership'], refusal=True)
            graph(label + '-old-flow-pool6-refusal', pool_path, ['--execution-flow-dag'], refusal=True)
            graph(label + '-all-old-flags-pool6-refusal', pool_path,
                  ['--query-cpu-ownership', '--execution-flow-dag', '--observed-task-dag',
                   '--cache-call-provenance', '--dispatch-result-provenance'], refusal=True)
            require(all(row.get('execution_flow_dag') is None for row in combined),
                    'pool6 combined flags fabricated a v1 flow projection')
            output, artifact = evidence / (label + '-compact.json'), evidence / (label + '-artifact.json')
            command = [sys.executable, str(Path(__file__).with_name('frontend_mapping.py')),
                       '--capture', str(directory), '--justrust', str(consumer), '--large-capture',
                       '--latch-readiness-dag', '--graph-output', str(artifact), '--output', str(output)]
            _, analyzer_record = execute(label + '-analyzer', command, parse_json=False)
            summary = json.loads(output.read_bytes())
            require(len(summary['captures']) == 1, 'actual analyzer capture inventory mismatch')
            compact = summary['captures'][0]
            require(all(key not in compact for key in ('query_cpu_ownership', 'cache_call_provenance',
                        'dispatch_result_provenance', 'observed_task_dag', 'execution_flow_dag')), 'compact flow implied unrelated diagnostics')
            artifact_values = json.loads(artifact.read_bytes())
            # Reproduce only the analyzer's neutral source-name routing. Original
            # scandir order assigns neutral identities, native graph orders them.
            # Copy exact original bytes, never rewrite receipt JSON or originals.
            reference = evidence / (label + '-neutral-reference')
            mapping = neutral_snapshot(directory, reference)
            mapping_bytes = json.dumps(mapping, sort_keys=True).encode()
            private_write(evidence / (label + '-neutral-before.json'), mapping_bytes)
            reference_values, reference_record = graph(label + '-artifact-reference', reference,
                                                       ['--latch-readiness-dag'])
            check_report_inventory(artifact_values, reference_values)
            require(artifact.read_bytes() == (evidence / (label + '-artifact-reference.stdout')).read_bytes(),
                    'analyzer companion differs from exact independently replayed neutral stdout')
            check_neutral_snapshot(directory, reference, mapping)
            private_write(evidence / (label + '-neutral-after.json'), mapping_bytes)
            reference_record.update(exact_analyzer_stdout_checked=True,
                                    original_byte_snapshot_mapping_sha256=hashlib.sha256(mapping_bytes).hexdigest())
            artifact_flow = pool_flow(artifact_values)
            check_flow(artifact_flow, expected, pool)
            check_compact(compact['latch_readiness_dag'], artifact_flow)
            require(artifact_flow == public, 'analyzer companion changed original flow assessment')
            require(artifact.stat().st_mode & 0o777 == output.stat().st_mode & 0o777 == 0o600,
                    'actual analyzer artifacts are not mode0600')
            require(summary['graph_output']['sha256'] == digest(artifact)
                    and summary['graph_output']['bytes'] == artifact.stat().st_size,
                    'exact companion stdout hash/size mismatch')
            artifact_hash, output_hash = digest(artifact), digest(output)
            execute(label + '-output-collision', command, refusal=True)
            require(digest(artifact) == artifact_hash and digest(output) == output_hash,
                    'collision overwrote existing report or full graph')
            collision_command = command[:-1] + [str(evidence / (label + '-collision-unused.json'))]
            execute(label + '-artifact-collision', collision_command, refusal=True)
            require(digest(artifact) == artifact_hash and not Path(collision_command[-1]).exists(),
                    'artifact collision overwrote stdout or created report')
            analyzer_record.update(compact_rankings_artifact_hash_mode_and_collisions_checked=True,
                                   artifact_sha256=artifact_hash, artifact_bytes=artifact.stat().st_size,
                                   compact_sha256=output_hash)
            if label == 'readiness-warm-1':
                warm = query_path, query, pool_path, pool

        query_path, query, pool_path, pool = warm
        for variant in ('missing-query', 'clock-mismatch', 'duplicate-pool', 'cross-producer-collision'):
            directory = evidence / (variant + '-copy')
            directory.mkdir(mode=0o700)
            private_write(directory / 'pool.json', pool_path.read_bytes())
            if variant != 'missing-query':
                changed = copy.deepcopy(query)
                if variant == 'clock-mismatch':
                    changed['execution_clock']['pool_observer'] += 1
                private_write(directory / 'query.json', json.dumps(changed).encode())
            if variant == 'duplicate-pool':
                private_write(directory / 'pool-copy.json', pool_path.read_bytes())
            if variant == 'cross-producer-collision':
                scheduler = dict(schema=1, producer='cargo-scheduler', scope='cargo-unit-scheduler',
                                 capture_id=pool['capture_id'], declared_nodes=0, declared_edges=0,
                                 structural_complete=False, full_pipeline_complete=False,
                                 issues=[], opaque_boundaries=['native'], events=[], nodes=[], edges=[])
                private_write(directory / 'scheduler.json', json.dumps(scheduler).encode())
            values, record = graph(variant, directory, ['--latch-readiness-dag'])
            no_full(values)
            pool_rows = [row for row in values if row['producer'] == 'rustc-thread-pool-join']
            require(len(pool_rows) == (2 if variant == 'duplicate-pool' else 1),
                    'negative replay omitted an original pool or a duplicate copy')
            for row in pool_rows:
                partial = row['latch_readiness_dag']
                scoped(partial)
                require(partial['records_complete'] is False and partial['issues'],
                        'unqualified negative flow acquired complete authority')
                require(partial['known_query_cpu_ns'] is None
                        and all(step['query_name'] is None for step in partial['longest_known_chain_steps']),
                        'unqualified negative flow acquired named query CPU')
                if variant in ('duplicate-pool', 'cross-producer-collision'):
                    require(all(partial[key] == 0 for key in COUNTS)
                            and all(partial[key] is None for key in COSTS)
                            and partial['longest_known_chain_path'] == partial['longest_known_chain_steps'] == []
                            and partial['ready_records_complete'] is False
                            and partial['wait_records_complete'] is False,
                            'colliding source retained unavailable flow authority')
                else:
                    check_unqualified(partial, query, pool)
            record['unknown_query_cpu_no_names_or_false_completeness_checked'] = True
    finally:
        after = inventory()
        binary_after = dict(bytes=consumer.stat().st_size, sha256=digest(consumer))
        private_write(evidence / 'inputs-after.json', json.dumps(after, sort_keys=True).encode())
        private_write(evidence / 'binary-after.json', json.dumps(binary_after, sort_keys=True).encode())
        require(before == after, 'accepted original inputs changed during replay')
        require(binary_before == binary_after, 'consumer binary changed during replay')

    receipt = dict(schema=1, scope='original-native-latch-readiness-public-acceptance-not-a-benchmark',
                   consumer_sha256=binary_before['sha256'], cases=cases, all_checks_passed=True,
                   original_files_unchanged=len(before), original_inventory_sha256=hashlib.sha256(
                       json.dumps(before, sort_keys=True).encode()).hexdigest(),
                   full_pipeline_complete=False, cpu_work_ns=None, execution_span_ns=None,
                   theoretical_parallelism=None,
                   limitations=['Replays completed real compiler captures, not a compiler rerun or benchmark.',
                                'Known retained CPU chains are not full span, work/span ratios or thread ceilings.',
                                'Full query identities and witnesses remain only in private evidence.'])
    private_write(evidence / 'summary.json', (json.dumps(receipt, indent=2) + '\n').encode())
    print(json.dumps(receipt, indent=2))


if __name__ == '__main__':
    main()
