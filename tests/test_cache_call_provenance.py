#!/usr/bin/env python3
"""Independent retained-byte acceptance for opt-in cache-call provenance.

Unit tests need no compiler or consumer. Public acceptance is opt-in via
JR_CACHE_CALL_CAPTURE_ROOT and JR_CACHE_CALL_JUSTRUST, after consumer freeze.
Original capture files and frozen adjacent assessments are never modified.
"""
import collections
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


REASONS = (
    'feed_request', 'interrupted_request', 'missing_request_terminal',
    'ambiguous_cache_hits', 'no_cache_hit', 'incompatible_cache_hit_route',
    'missing_context_or_consumer', 'capture_incomplete', 'no_prior_producer',
    'producer_published_too_late', 'job_endpoint_unavailable',
    'admission_authority_incomplete',
)


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def oracle(query, *, admission_authority=True):
    """Original-stream index oracle, deliberately independent of dep_read outcome.

    Admitted endpoints, not completion markers, authorize publishing jobs.
    Publication availability is not an execution dependency or completion proof.
    """
    reads = query['query_reads']
    events = reads['events']
    admitted = {(e['context_id'], e['job_id']) for e in query['events']
                if e['kind'] == 'admitted'}
    starts = {}
    terminals = {}
    hits = collections.defaultdict(list)
    publications = collections.defaultdict(list)
    for index, event in enumerate(events):
        kind = event['kind']
        if kind == 'request_started':
            starts[event['request_id']] = (index, event)
        elif kind in ('request_finished', 'request_interrupted'):
            terminals[event['request_id']] = (index, event)
        elif kind == 'cache_hit':
            hits[event['request_id']].append((index, event))
        elif kind == 'result_produced':
            publications[(event['context_id'], event['dep_node_index'])].append((index, event))
    reasons = dict.fromkeys(REASONS, 0)
    relations = []
    for request_id, (start_index, start) in sorted(starts.items()):
        terminal = terminals.get(request_id)
        request_hits = hits.get(request_id, [])
        reason = None
        if start['mode'] == 'feed':
            reason = 'feed_request'
        elif terminal is None:
            reason = 'missing_request_terminal'
        elif terminal[1]['kind'] == 'request_interrupted':
            reason = 'interrupted_request'
        elif len(request_hits) > 1:
            reason = 'ambiguous_cache_hits'
        elif not request_hits:
            reason = 'no_cache_hit'
        elif not ((start['mode'] == 'get' and terminal[1]['outcome'] == 'value') or
                  (start['mode'] in ('ensure_ok', 'ensure_result') and
                   terminal[1]['outcome'] == 'ensured')):
            reason = 'incompatible_cache_hit_route'
        else:
            hit_index, hit = request_hits[0]
            assert start_index < hit_index < terminal[0], 'request interval must contain hit'
            context, consumer = start['context_id'], start['consumer_job_id']
            assert (hit['context_id'], hit['consumer_job_id']) == (context, consumer)
            if context is None or consumer is None:
                reason = 'missing_context_or_consumer'
            elif not reads['records_complete'] or reads['lost_events']:
                reason = 'capture_incomplete'
            else:
                producers = publications.get((context, hit['dep_node_index']), [])
                prior = [(i, e) for i, e in producers if i < hit_index]
                if not prior:
                    reason = ('producer_published_too_late' if producers else 'no_prior_producer')
                elif len(prior) != 1:
                    raise AssertionError('multiple publishing results require strict input rejection')
                else:
                    publication_index, producer = prior[0]
                    producer_job = producer['producer_job_id']
                    if (context, consumer) not in admitted or (context, producer_job) not in admitted:
                        reason = 'job_endpoint_unavailable'
                    else:
                        relations.append({
                            'request_id': request_id, 'context_id': context,
                            'consumer_job_id': consumer, 'result_id': producer['result_id'],
                            'producer_job_id': producer_job, 'dep_node_index': hit['dep_node_index'],
                            'request_start_event_index': start_index,
                            'publication_event_index': publication_index,
                            'cache_hit_event_index': hit_index,
                            'request_terminal_event_index': terminal[0],
                        })
        if reason:
            reasons[reason] += 1
    if not admission_authority:
        reasons['admission_authority_incomplete'] += len(relations)
        relations = []
    return {
        'scope': 'observed-cache-result-availability-not-execution-dag',
        'records_complete': reads['records_complete'],
        'observed_requests': len(starts),
        'observed_cache_hits': sum(map(len, hits.values())),
        'orphan_cache_hits': sum(len(v) for k, v in hits.items() if k not in starts),
        'resolved_requests': len(relations),
        'unresolved_requests': len(starts) - len(relations),
        'distinct_publishing_job_relations': len({(r['context_id'], r['producer_job_id'],
                                                  r['consumer_job_id']) for r in relations}),
        'lost_events': reads['lost_events'], 'unresolved_reasons': reasons,
        'relations': relations,
    }


def small_query():
    def event(kind, **fields):
        return dict(kind=kind, **fields)
    return {
        'events': [event('admitted', context_id=1, job_id=1),
                   event('admitted', context_id=1, job_id=2)],
        'query_reads': {'records_complete': True, 'lost_events': 0, 'events': [
            event('request_started', request_id=1, context_id=1, consumer_job_id=1, mode='get'),
            event('result_produced', context_id=1, dep_node_index=7, result_id=1, producer_job_id=2),
            event('cache_hit', request_id=1, context_id=1, consumer_job_id=1, dep_node_index=7),
            event('request_finished', request_id=1, outcome='value'),
        ]},
    }


class OracleTests(unittest.TestCase):
    def test_ongoing_producer_hit_needs_no_completion(self):
        query = small_query()
        assessment = oracle(query)
        self.assertEqual(assessment['resolved_requests'], 1)
        self.assertEqual(assessment['relations'][0]['publication_event_index'], 1)
        self.assertEqual(assessment['relations'][0]['cache_hit_event_index'], 2)
        self.assertFalse(any(e['kind'] == 'completed' for e in query['events']))

    def test_future_publication_never_binds(self):
        query = small_query()
        events = query['query_reads']['events']
        events[1], events[2] = events[2], events[1]
        assessment = oracle(query)
        self.assertEqual(assessment['relations'], [])
        self.assertEqual(assessment['unresolved_reasons']['producer_published_too_late'], 1)

    def test_disabled_read_is_not_cache_hit_authority(self):
        query = small_query()
        query['query_reads']['events'].insert(3, {'kind': 'dep_read', 'outcome': 'disabled'})
        self.assertEqual(oracle(query)['resolved_requests'], 1)

    def test_ambiguous_hits_and_green_ensure_are_not_relations(self):
        query = small_query()
        query['query_reads']['events'].insert(3, dict(query['query_reads']['events'][2]))
        self.assertEqual(oracle(query)['unresolved_reasons']['ambiguous_cache_hits'], 1)
        query = small_query()
        query['query_reads']['events'][0]['mode'] = 'ensure_ok'
        query['query_reads']['events'][-1]['outcome'] = 'green_ensure'
        self.assertEqual(oracle(query)['unresolved_reasons']['incompatible_cache_hit_route'], 1)

    def test_admission_downgrade_preserves_request_partition(self):
        assessment = oracle(small_query(), admission_authority=False)
        self.assertTrue(assessment['records_complete'])
        self.assertEqual(assessment['relations'], [])
        self.assertEqual(assessment['resolved_requests'], 0)
        self.assertEqual(assessment['unresolved_requests'], 1)
        self.assertEqual(assessment['unresolved_reasons']['admission_authority_incomplete'], 1)

    def test_self_job_is_an_event_relation_not_a_dag_edge(self):
        query = small_query()
        query['query_reads']['events'][1]['producer_job_id'] = 1
        assessment = oracle(query)
        self.assertEqual(assessment['resolved_requests'], 1)
        self.assertEqual(assessment['relations'][0]['producer_job_id'],
                         assessment['relations'][0]['consumer_job_id'])

    def test_orphan_hits_are_separate_from_request_partition(self):
        query = small_query()
        orphan = dict(query['query_reads']['events'][2], request_id=99)
        query['query_reads']['events'].append(orphan)
        assessment = oracle(query)
        self.assertEqual(assessment['orphan_cache_hits'], 1)
        self.assertEqual(assessment['observed_cache_hits'], 2)
        self.assertEqual(assessment['observed_requests'], 1)
        self.assertEqual(assessment['resolved_requests'], 1)

    def test_root_consumer_and_loss_remain_unresolved(self):
        query = small_query()
        query['query_reads']['events'][0]['consumer_job_id'] = None
        query['query_reads']['events'][2]['consumer_job_id'] = None
        self.assertEqual(oracle(query)['unresolved_reasons']['missing_context_or_consumer'], 1)
        query = small_query()
        query['query_reads']['records_complete'] = False
        query['query_reads']['lost_events'] = 1
        self.assertEqual(oracle(query)['unresolved_reasons']['capture_incomplete'], 1)


@unittest.skipUnless(os.environ.get('JR_CACHE_CALL_CAPTURE_ROOT') and
                     os.environ.get('JR_CACHE_CALL_JUSTRUST'), 'retained acceptance opt-in')
class RetainedAcceptance(unittest.TestCase):
    def test_real_cold_and_warm_exact_replay(self):
        source = Path(os.environ['JR_CACHE_CALL_CAPTURE_ROOT']).resolve()
        evidence = Path(os.environ.get('JR_CACHE_CALL_EVIDENCE_DIR',
                                       os.environ.get('JCODE_SCRATCH_DIR', '.')))
        evidence.mkdir(parents=True, exist_ok=True)
        root = Path(tempfile.mkdtemp(prefix='cache-call-acceptance-', dir=evidence))
        client = root / 'justrust-consumer'
        shutil.copy2(os.environ['JR_CACHE_CALL_JUSTRUST'], client)
        print(json.dumps({'evidence_dir': str(root), 'consumer_sha256': digest(client)}), flush=True)
        captures = [(source / 'timed-cold-t4-1-capture', True),
                    (source / 'timed-warm-t4-1-capture', False)]
        protected = [p for capture, _ in captures for p in capture.glob('*.json')]
        protected.extend(source / (capture.name.removesuffix('-capture') + '-assessment.json')
                         for capture, _ in captures)
        original = {str(p): digest(p) for p in protected}
        (root / 'input-sha256.json').write_text(json.dumps(original, indent=2))

        def graph(capture, label, flags):
            command = [str(client), 'graph', str(capture), '--json', *flags]
            run = subprocess.run(command, text=True, capture_output=True)
            (root / (label + '.json')).write_text(json.dumps({
                'command': command, 'returncode': run.returncode,
                'stdout': run.stdout, 'stderr': run.stderr}, indent=2))
            self.assertEqual(run.returncode, 0, run.stderr)
            return json.loads(run.stdout)

        try:
            for capture, large in captures:
                label = capture.name.removesuffix('-capture')
                flags = ['--large-capture'] if large else []
                frozen = json.loads((source / (label + '-assessment.json')).read_text())
                default = graph(capture, label + '-default', flags)
                self.assertEqual(default, frozen, 'default report changed')
                opted = graph(capture, label + '-opted', flags + ['--cache-call-provenance'])
                for row in opted:
                    self.assertFalse(row['full_pipeline_complete'])
                    self.assertIsNone(row['cpu_work_ns'])
                    self.assertIsNone(row['execution_span_ns'])
                query_path = next(p for p in capture.glob('*.json')
                                  if not p.name.endswith('-pool.json'))
                query = json.loads(query_path.read_text())
                expected = oracle(query)
                public = next(r for r in opted if r['producer'] == 'rustc-query-admission')
                self.assertEqual(public['cache_call_provenance'], expected)
                replay = json.loads(json.dumps(opted))
                for row in replay:
                    row.pop('cache_call_provenance', None)
                self.assertEqual(replay, default, 'opt-in changed preexisting graph authority')
                self.assertEqual(sum(expected['unresolved_reasons'].values()),
                                 expected['unresolved_requests'])
                self.assertGreater(expected['resolved_requests'], 0)
                (root / (label + '-oracle.json')).write_text(json.dumps(expected, indent=2))
                print(json.dumps({'label': label, **{k: v for k, v in expected.items()
                                                    if k != 'relations'}}), flush=True)
        finally:
            after = {str(p): digest(p) for p in protected}
            (root / 'input-sha256-after.json').write_text(json.dumps(after, indent=2))
            self.assertEqual(after, original, 'original capture bytes changed')


if __name__ == '__main__':
    unittest.main()
