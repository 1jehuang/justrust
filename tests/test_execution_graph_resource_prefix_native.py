#!/usr/bin/env python3
"""Separate real resource-prefix native gate, never builds the compiler.

Opt in with JR_RESOURCE_RUSTC pointing at an already rebuilt private driver.
JR_RESOURCE_STAGE0, JR_RESOURCE_METADATA_SYSROOT, JR_RESOURCE_RUN_SYSROOT and
JR_RESOURCE_EVIDENCE_DIR select existing artifacts/evidence. Matching full private
metadata is mandatory. Existing selected-six and legacy-nine gates are untouched.

OFF prepares the SAME full native journal as ON, but has no query publication
sink. This is publication-only neutrality, NOT a zero-telemetry-overhead claim.
Rare actual IO faults, blocked-query panic and compiler identity exhaustion are
explicitly deferred, not replaced with synthetic positive native evidence.
"""
import copy
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time
import unittest

from test_execution_graph_selected_admission_native import MACRO, prepare_capture, sha256

RESOURCE = 'rustc-frontend-allowance'
ENDORSEMENT = 'rustc-frontend-admission-attachment'
QUERY = 'rustc-query-admission'
POOL = 'rustc-thread-pool-join'
TUPLE_FIELDS = ('attachment_capture_id', 'attachment_id', 'controller_id',
                'resource_capture_id', 'query_capture_id', 'query_root_namespace',
                'pool_capture_id', 'pool_observer')
INPUT = '''observer::observe!();
fn resource_target() -> u64 { 42 }
fn main() {
    println!("value={}; private_mode_arg={}", resource_target(), PRIVATE_MODE_ARG);
    println!("argv={}", OBSERVED_ARGS);
    println!("env_fingerprint={}", OBSERVED_ENV);
}
'''


def events(resource):
    return [(transaction, event) for transaction in resource['transactions']
            for event in transaction['events']]


def documents(directory):
    return {p.name: json.loads(p.read_text()) for p in directory.glob('*.json')}


@unittest.skipUnless(os.environ.get('JR_RESOURCE_RUSTC'), 'requires rebuilt JR_RESOURCE_RUSTC')
class NativeResourcePrefix(unittest.TestCase):
    def logged(self, command, label, *, pass_fds=(), expected=0):
        command = [str(a) for a in command]
        started = time.monotonic()
        try:
            result = subprocess.run(command, cwd=self.root, env=self.env,
                                    pass_fds=pass_fds, capture_output=True,
                                    text=True, timeout=125)
        except subprocess.TimeoutExpired as error:
            (self.root / (label + '.json')).write_text(json.dumps({
                'command': command, 'status': 'insufficientexercise',
                'reason': 'deadline is NOT proof of query wait or native Pending',
                'stdout': str(error.stdout), 'stderr': str(error.stderr)}, indent=2))
            self.fail('insufficientexercise: native fixture deadline, evidence ' + str(self.root))
        (self.root / (label + '.json')).write_text(json.dumps({
            'command': command, 'exit_code': result.returncode,
            'elapsed_s': time.monotonic() - started,
            'stdout': result.stdout, 'stderr': result.stderr}, indent=2))
        if expected is not None:
            self.assertEqual(result.returncode, expected, result.stdout + result.stderr)
        return result

    def setUp(self):
        base = os.environ.get('JR_RESOURCE_EVIDENCE_DIR') or os.environ.get('JCODE_SCRATCH_DIR')
        if base:
            Path(base).mkdir(parents=True, exist_ok=True)
        self.root = Path(tempfile.mkdtemp(prefix='native-resource-prefix-', dir=base))
        print(json.dumps({'test': self._testMethodName, 'evidence_dir': str(self.root),
                          'retained': True}), flush=True)
        self.rustc = Path(os.environ['JR_RESOURCE_RUSTC']).resolve(strict=True)
        self.stage = self.rustc.parent.parent
        self.sysroot = Path(os.environ.get('JR_RESOURCE_RUN_SYSROOT', self.stage)).resolve(strict=True)
        host = self.stage.parent
        self.stage0 = Path(os.environ.get('JR_RESOURCE_STAGE0', host / 'stage0/bin/rustc')).resolve(strict=True)
        self.metadata = Path(os.environ.get('JR_RESOURCE_METADATA_SYSROOT', host / 'stage0-sysroot')).resolve(strict=True)
        self.justrust = os.environ.get('JR_RESOURCE_JUSTRUST') or shutil.which('justrust')
        self.assertTrue(self.justrust, 'actual resource graph consumer required')
        driver = list((self.stage / 'lib').glob('librustc_driver-*.so'))
        self.assertEqual(len(driver), 1, 'require unique already rebuilt driver')
        driver = driver[0]
        metadata = list(self.metadata.glob('lib/rustlib/*/lib/' + driver.with_suffix('.rmeta').name))
        self.assertEqual(len(metadata), 1, 'matching full private metadata required')
        self.env = os.environ.copy()
        for key in ('JUSTRUST_EXECUTION_GRAPH_DIR', 'RUSTC_WRAPPER', 'RUSTC_WORKSPACE_WRAPPER',
                    'RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS', 'CARGO_MAKEFLAGS', 'MAKEFLAGS', 'MFLAGS'):
            self.env.pop(key, None)
        self.env['RUSTC_BOOTSTRAP'] = '1'
        self.env['LD_LIBRARY_PATH'] = os.pathsep.join([
            str(driver.parent), str(metadata[0].parent), str(self.metadata / 'lib'),
            self.env.get('LD_LIBRARY_PATH', '')])
        fixture = Path(__file__).resolve().parent / 'fixtures/execution_graph_resource_prefix.rs'
        helper = Path(__file__).with_name('test_execution_graph_selected_admission_native.py')
        for source in (fixture, Path(__file__), helper):
            shutil.copyfile(source, self.root / source.name)
        (self.root / 'private-artifacts.json').write_text(json.dumps({
            str(path): {'sha256': sha256(path), 'mtime_ns': path.stat().st_mtime_ns}
            for path in (self.rustc, self.stage0, driver, metadata[0], fixture, Path(__file__), helper)}, indent=2))
        self.callback_build = [self.stage0, '--edition=2024', '--sysroot', self.metadata,
                               '-Ldependency=' + str(metadata[0].parent),
                               '-Ldependency=' + str(driver.parent),
                               '--extern', 'rustc_driver=' + str(metadata[0]),
                               '--extern', 'rustc_driver=' + str(driver), '-Cprefer-dynamic']
        self.fixture = fixture
        self.macro = self.root / 'libobserver.so'
        macro = self.root / 'observer.rs'
        macro.write_text(MACRO)
        self.logged([self.rustc, '--sysroot', self.sysroot, '--crate-name=observer',
                     '--crate-type=proc-macro', macro, '-o', self.macro], 'macro-build')
        self.source = self.root / 'input.rs'
        self.source.write_text(INPUT)
        self.artifact = self.root / 'input-bin'

    def assess(self, sink, label, *, strict=True):
        result = self.logged([self.justrust, 'graph', sink, '--json'], label,
                             expected=0 if strict else None)
        if result.returncode:
            return None
        result = json.loads(result.stdout)
        self.assertIsInstance(result, list, 'requires actual public graph JSON route')
        return result

    def resource_assessment(self, assessment):
        found = [row for row in assessment if row.get('producer') == RESOURCE]
        self.assertEqual(len(found), 1, assessment)
        return found[0]['resource']

    def attachment_assessment(self, assessment, attachment):
        collectors = [row['resource_attachments'] for row in assessment
                      if 'resource_attachments' in row]
        self.assertEqual(len(collectors), 1, assessment)
        links = [row for row in collectors[0]['attachments']
                 if row['attachment_capture_id'] == attachment['attachment_capture_id']]
        self.assertEqual(len(links), 1, collectors)
        return links[0]

    def diagnostic(self, result, *, qualified, local, pregrant, capacity):
        prefix = 'resource-prefix-evidence: '
        rows = [json.loads(line[len(prefix):]) for line in result.stderr.splitlines()
                if line.startswith(prefix)]
        self.assertEqual(len(rows), 1, 'insufficientexercise: no positive native controller facts\n' + result.stderr)
        row = rows[0]
        self.assertEqual(row['source'], 'local' if local else 'helper')
        self.assertEqual(row['cut'], 'pregrant' if pregrant else 'postresume')
        self.assertEqual(row['transaction_capacity'], capacity)
        self.assertEqual(row['qualified'], qualified)
        self.assertIs(row['publication_only_off'], True)
        for fact in ('actual_query_release', 'producer_completed', 'native_pending', 'returned_after_source'):
            self.assertIs(row[fact], True, row)
        self.assertIs(row['returned_before_source'], False, row)
        self.assertGreater(row['frozen_requests'], 0)
        self.assertGreater(row['frozen_acks'], 0)
        self.assertGreater(row['frozen_returns'], 0)
        if qualified:
            self.assertGreater(row['wait_id'], 0)
        else:
            self.assertEqual(row['wait_id'], 0)
        return row

    def run_pair(self, *, local=False, pregrant=False, capacity=4096):
        cfg = []
        if local:
            cfg.append('--cfg=resource_local_transfer')
        if pregrant:
            cfg.append('--cfg=resource_cut_pregrant')
        if capacity == 0:
            cfg.append('--cfg=resource_cap_zero')
        elif capacity == 1:
            cfg.append('--cfg=resource_cap_tiny')
        else:
            self.assertEqual(capacity, 4096)
        callback = self.root / 'native-resource-callback'
        self.logged(self.callback_build + cfg + [self.fixture, '-o', callback], 'callback-build')
        command = [callback, '--sysroot', self.sysroot, '-Zthreads=2', '--edition=2024',
                   '--crate-name=resource_input', '--extern', 'observer=' + str(self.macro),
                   self.source, '-o', self.artifact]
        off = self.logged(command, 'off')
        off_facts = self.diagnostic(off, qualified=False, local=local, pregrant=pregrant, capacity=capacity)
        off_hash = sha256(self.artifact)
        off_output = self.logged([self.artifact], 'execute-off').stdout
        sink = self.root / 'capture'
        ticket, directory = prepare_capture(sink)
        try:
            on = self.logged(command, 'on', pass_fds=(ticket, directory))
        finally:
            os.close(ticket)
            os.close(directory)
        on_facts = self.diagnostic(on, qualified=True, local=local, pregrant=pregrant, capacity=capacity)
        self.assertEqual(off_hash, sha256(self.artifact), 'publication changed artifact bytes')
        on_output = self.logged([self.artifact], 'execute-on').stdout
        self.assertEqual(off_output, on_output, 'actual macro argv/environment changed')
        self.assertIn('value=42; private_mode_arg=false', on_output)
        strip = lambda text: '\n'.join(line for line in text.splitlines()
                                      if not line.startswith('resource-prefix-evidence: '))
        self.assertEqual(strip(off.stderr), strip(on.stderr), 'publication changed compiler diagnostics')
        data = documents(sink)
        self.assertTrue(data, 'production finalizer did not publish native evidence')
        by_producer = lambda producer: [(name, row) for name, row in data.items()
                                        if row.get('producer') == producer]
        publication_inventory = {name: {'producer': row.get('producer'),
            'capture_id': row.get('capture_id'), 'sha256': sha256(sink / name)}
            for name, row in data.items()}
        self.assertEqual(len(by_producer(RESOURCE)), 1, publication_inventory)
        self.assertEqual(len(by_producer(QUERY)), 1, publication_inventory)
        self.assertEqual(len(by_producer(POOL)), 1, publication_inventory)
        resource_name, resource = by_producer(RESOURCE)[0]
        query_name, query = by_producer(QUERY)[0]
        pool_name, pool = by_producer(POOL)[0]
        endorsements = by_producer(ENDORSEMENT)
        self.assertEqual(len(endorsements), 3, data)
        self.assertEqual({row['role'] for _, row in endorsements}, {'query', 'pool', 'resource'})
        self.assertEqual(len({row['capture_id'] for _, row in endorsements}), 3)
        attachment = endorsements[0][1]['attachment']
        self.assertEqual(set(attachment), set(TUPLE_FIELDS))
        for _, row in endorsements:
            self.assertEqual(row['schema'], 1)
            self.assertEqual(row['scope'], 'selected-resource-query-pool-attachment')
            self.assertEqual(row['attachment'], attachment)
            self.assertIs(row['sealed'], True)
            self.assertEqual(row['expected_endorsements'], 1)
            self.assertEqual(row['endorsement_allocator_baseline'], 0)
            self.assertEqual(row['endorsement_allocator_high_water'], 1)
            self.assertEqual(row['lost_events'], 0)
            self.assertIs(row['loss_overflowed'], False)
        self.assertEqual(resource['schema'], 1)
        self.assertEqual(resource['scope'], 'selected-ticketed-grant-decisions')
        self.assertEqual(resource['capture_state'], 'sealed_prefix')
        self.assertEqual(resource['cut_reason'], 'controller_checkpoint')
        self.assertEqual(resource['expected_attachments'], 1)
        self.assertEqual(resource['attachment_capture_id'], attachment['attachment_capture_id'])
        self.assertEqual(resource['controller_id'], attachment['controller_id'])
        self.assertEqual(resource['capture_id'], attachment['resource_capture_id'])
        self.assertIsInstance(resource['controller_id'], str)
        self.assertNotEqual(resource['controller_id'], resource['capture_id'])
        self.assertEqual(query['schema'], 4)
        self.assertEqual(pool['schema'], 2)
        self.assertEqual(query['capture_id'], attachment['query_capture_id'])
        self.assertEqual(pool['capture_id'], attachment['pool_capture_id'])
        self.assertEqual(pool['observer'], attachment['pool_observer'])
        self.assertEqual(query['task_bindings']['pool']['capture_id'], pool['capture_id'])
        self.assertEqual(query['task_bindings']['pool']['observer'], pool['observer'])
        self.assertEqual(pool['query_capture_id'], query['capture_id'])
        for key in ('producer_closed', 'resource_full_complete', 'full_pipeline_complete', 'raw_io_coverage'):
            self.assertIs(resource[key], False)
        self.assertIsNone(resource['cpu_work_ns'])
        self.assertIsNone(resource['execution_span_ns'])
        self.assertIsNone(resource['physical_callbacks_inflight'])
        self.assertEqual(resource['end']['counters']['requests'], on_facts['frozen_requests'])
        self.assertEqual(resource['end']['counters']['acks'], on_facts['frozen_acks'])
        self.assertEqual(resource['end']['counters']['acquire_returns'], on_facts['frozen_returns'])
        self.assertGreater(resource['end']['counters']['events_attempted'], 0)
        assessment = self.assess(sink, 'assessment')
        assessed = self.resource_assessment(assessment)
        state = dict(sink=sink, data=data, resource_name=resource_name, query_name=query_name,
                     pool_name=pool_name, resource=resource, query=query, pool=pool,
                     endorsements=endorsements, attachment=attachment, assessment=assessment,
                     assessed=assessed, link=self.attachment_assessment(assessment, attachment),
                     wait_id=on_facts['wait_id'])
        if capacity < 4096:
            self.assertIs(resource['records_complete'], False)
            self.assertGreater(resource['lost_transactions'], 0)
            self.assertGreater(resource['lost_events'], 0)
            self.assertGreater(resource['end']['counters']['transactions_attempted'], len(resource['transactions']))
            self.assertIs(assessed['prefix_records_complete'], False)
            self.assertFalse(state['link']['cross_wait_edges'])
        else:
            self.assertIs(resource['records_complete'], True)
            self.assertEqual(resource['lost_transactions'], 0)
            self.assertEqual(resource['lost_events'], 0)
            self.assertIs(assessed['prefix_records_complete'], True)
            self.assertIs(assessed['prefix_protocol_valid'], True)
            self.assertEqual(state['link']['state'], 'validated')
            self.check_target(state, local=local, pregrant=pregrant)
        (self.root / 'comparison.json').write_text(json.dumps({
            'off': off_facts, 'on': on_facts, 'command': [str(a) for a in command],
            'identical_environment': True, 'publication_only_off': True,
            'full_telemetry_disabled_off': False, 'artifact_sha256': off_hash,
            'callback_sha256': sha256(callback), 'macro_sha256': sha256(self.macro),
            'receipt_sha256': {p.name: sha256(p) for p in sink.glob('*.json')},
            'macro_output': on_output,
            'deferred': ['actual-owned-IO-fault', 'blocked-query-panic', 'compiler-identity-exhaustion'],
            'full_mapping_claim': False, 'default_promotion': False}, indent=2))
        return state

    def check_target(self, state, *, local, pregrant):
        resource, query, pool = state['resource'], state['query'], state['pool']
        rows = events(resource)
        requests = [(tx, event) for tx, event in rows if event['kind'] == 'request'
                    and event['context'] is not None
                    and event['context']['phase'] == 'query_wait_reacquire'
                    and event['context']['wait_id'] == state['wait_id']]
        self.assertEqual(len(requests), 1, rows)
        request = requests[0][1]
        state['target_request'] = request['request_id']
        self.assertEqual(request['context']['query_root_namespace'], state['attachment']['query_root_namespace'])
        self.assertEqual(request['context']['attachment_id'], state['attachment']['attachment_id'])
        self.assertIsNone(request['unavailable_reason'])
        self.assertTrue(any(e['kind'] == 'pending' and e['request_id'] == state['target_request']
                            for _, e in rows), 'insufficientexercise: no recorded actual Pending')
        stages = {event['kind']: event for event in query['wait_events']
                  if event['wait_id'] == state['wait_id']}
        self.assertEqual(set(stages), {'registered', 'notified', 'condvar_returned', 'resumed'})
        times = [stages[k]['at_ns'] for k in ('registered', 'notified', 'condvar_returned', 'resumed')]
        self.assertEqual(times, sorted(times))
        grants = [(tx, e) for tx, e in rows if e['kind'] == 'grant'
                  and e['request_id'] == state['target_request']]
        if pregrant:
            self.assertFalse(grants, 'pregrant cut included the target grant')
            self.assertEqual(resource['end']['state']['pending'], 1)
            self.assertFalse(any(e['kind'] in ('ack', 'acquire_returned')
                                 and e['request_id'] == state['target_request'] for _, e in rows))
            self.assertIs(state['assessed']['closed_lifecycle'], False)
            self.assertIs(state['link']['context_links_complete'], False)
            self.assertFalse(self.target_edges(state['link'], state))
            return
        self.assertEqual(len(grants), 1, rows)
        grant_tx, grant = grants[0]
        state['target_grant'] = grant['grant_id']
        self.assertEqual(grant['source']['kind'], 'release' if local else 'helper')
        if local:
            source = [e for _, e in rows if e['kind'] == 'release_decision'
                      and e['release_id'] == grant['source']['release_id']]
            self.assertEqual(len(source), 1)
            self.assertEqual(source[0]['disposition'], dict(kind='transfer', grant_id=grant['grant_id'],
                                                           request_id=state['target_request']))
            # Actual join administrative release can be unqualified. Never label
            # all contextNone as administrative or infer a prior token holder.
            self.assertIn(source[0], grant_tx['events'])
        else:
            source = [e for _, e in rows if e['kind'] == 'helper_decision'
                      and e['callback_id'] == grant['source']['callback_id']]
            self.assertEqual(len(source), 1)
            self.assertEqual(source[0]['outcome'], dict(kind='accepted', grant_id=grant['grant_id'],
                                                       request_id=state['target_request']))
            self.assertIn(source[0], grant_tx['events'])
        acks = [e for _, e in rows if e['kind'] == 'ack' and e['request_id'] == state['target_request']]
        self.assertEqual(len(acks), 1)
        self.assertEqual(acks[0]['grant_id'], grant['grant_id'])
        state['target_ack'] = acks[0]['ack_id']
        returned = [e for _, e in rows if e['kind'] == 'acquire_returned'
                    and e['request_id'] == state['target_request']]
        self.assertEqual(len(returned), 1, 'Ack is NOT actual adapter AcquireReturned')
        self.assertEqual(returned[0]['grant_id'], grant['grant_id'])
        self.assertEqual(returned[0]['ack_id'], acks[0]['ack_id'])
        state['target_return'] = returned[0]['return_id']
        edges = self.target_edges(state['link'], state)
        self.assertIs(state['link']['context_links_valid'], True)
        self.assertEqual(len(edges), 1, state['link'])
        self.assertEqual(edges[0]['acquire_return_id'], state['target_return'])
        points = [e for e in query['task_bindings']['events']
                  if e['source']['kind'] == 'wait_resumed'
                  and e['source']['wait_id'] == state['wait_id']]
        self.assertEqual(len(points), 1)
        self.assertIsNotNone(points[0]['target'])
        target = points[0]['target']
        starts = {e['task']: e['at_ns'] for e in pool['events'] if e['kind'] == 'start'}
        finishes = {e['task']: e['at_ns'] for e in pool['events'] if e['kind'] == 'finish'}
        self.assertLessEqual(starts[target['task']], target['at_pool_ns'])
        self.assertLessEqual(target['at_pool_ns'], finishes[target['task']])

    def test_helper_postresume_and_negative_copies(self):
        state = self.run_pair()
        self.negative_copies(state)

    def test_local_postresume(self):
        state = self.run_pair(local=True)
        self.source_mutation(state)

    def test_helper_pregrant_prefix(self):
        self.run_pair(pregrant=True)

    def test_local_pregrant_prefix(self):
        self.run_pair(local=True, pregrant=True)

    def test_capacity_zero_native_progress(self):
        self.run_pair(capacity=0)

    def test_capacity_tiny_native_progress(self):
        self.run_pair(capacity=1)

    @staticmethod
    def target_edges(link, state):
        return [edge for edge in link['cross_wait_edges']
                if edge['resource_capture_id'] == state['resource']['capture_id']
                and edge['query_capture_id'] == state['query']['capture_id']
                and edge['wait_id'] == state['wait_id']]

    def mutated(self, state, label, mutate, *, preserve_edge=False):
        data = copy.deepcopy(state['data'])
        mutate(data)
        sink = self.root / ('mutation-' + label)
        sink.mkdir()
        for name, row in data.items():
            (sink / name).write_text(json.dumps(row))
        assessment = self.assess(sink, 'mutation-' + label, strict=False)
        if assessment is None:
            self.assertFalse(preserve_edge, 'optional binding invalidated whole route')
            return
        collectors = [row['resource_attachments'] for row in assessment
                      if 'resource_attachments' in row]
        self.assertEqual(len(collectors), 1)
        links = [link for link in collectors[0]['attachments']
                 if link['attachment_capture_id'] == state['attachment']['attachment_capture_id']]
        actual = [edge for link in links for edge in self.target_edges(link, state)]
        self.assertEqual(bool(actual), preserve_edge, (label, collectors))
        # Independently assess the same query/pool bytes without any resource
        # role. This checks altered query/pool inputs too, not merely originals.
        standalone = sink / 'standalone-query-pool'
        standalone.mkdir()
        for name in (state['query_name'], state['pool_name']):
            if name in data:
                (standalone / name).write_text(json.dumps(data[name]))
        independent = self.assess(standalone, 'independent-' + label, strict=False)
        if independent is not None:
            for original in independent:
                current = next(row for row in assessment
                    if row.get('capture_id') == original.get('capture_id')
                    and row.get('producer') == original.get('producer'))
                clean = lambda row: {k: v for k, v in row.items() if k != 'resource_attachments'}
                self.assertEqual(clean(current), clean(original), label)

    def source_mutation(self, state):
        def wrong_source(data):
            for _, event in events(data[state['resource_name']]):
                if event['kind'] == 'grant' and event['request_id'] == state['target_request']:
                    event['source'] = {'kind': 'idle_implicit'}
        self.mutated(state, 'wrong-native-source', wrong_source)

    def negative_copies(self, state):
        self.source_mutation(state)
        for name, endorsement in state['endorsements']:
            role = endorsement['role']
            self.mutated(state, 'missing-role-' + role, lambda data, n=name: data.pop(n))
            self.mutated(state, 'duplicate-role-' + role,
                         lambda data, n=name: data.update({'duplicate-' + n: copy.deepcopy(data[n])}))
            for field in TUPLE_FIELDS:
                def mismatch(data, n=name, f=field):
                    old = data[n]['attachment'][f]
                    data[n]['attachment'][f] = old + 1 if isinstance(old, int) else old + '-foreign'
                self.mutated(state, 'tuple-' + role + '-' + field, mismatch)
        for category in ('request', 'pending', 'grant', 'ack', 'acquire_returned',
                         'helper_issued', 'callback_registered', 'helper_decision', 'callback_complete'):
            if not any(event['kind'] == category for _, event in events(state['resource'])):
                # Sealed prefix need not contain helper completion. Do not
                # manufacture a callback endpoint or claim helper quiescence.
                self.assertIn(category, ('callback_complete',))
                continue
            def omit(data, kind=category):
                for transaction in data[state['resource_name']]['transactions']:
                    transaction['events'] = [event for event in transaction['events'] if event['kind'] != kind]
            self.mutated(state, 'missing-category-' + category, omit)
        for kind, field in (('ack', 'grant_id'), ('acquire_returned', 'ack_id')):
            def wrong_reference(data, k=kind, f=field):
                for _, event in events(data[state['resource_name']]):
                    if event['kind'] == k and event['request_id'] == state['target_request']:
                        event[f] += 100000
            self.mutated(state, 'wrong-reference-' + kind, wrong_reference)
        def foreign_context(data):
            for _, event in events(data[state['resource_name']]):
                if event['kind'] == 'request' and event['request_id'] == state['target_request']:
                    event['context']['query_root_namespace'] += 1
        self.mutated(state, 'foreign-context-same-numeric-wait', foreign_context)
        def missing_resumed(data):
            query = data[state['query_name']]
            query['wait_events'] = [e for e in query['wait_events']
                                    if not (e['kind'] == 'resumed' and e['wait_id'] == state['wait_id'])]
        self.mutated(state, 'missing-query-resumed', missing_resumed)
        def optional_point(data):
            for event in data[state['query_name']]['task_bindings']['events']:
                if event['source']['kind'] == 'wait_resumed' and event['source']['wait_id'] == state['wait_id']:
                    event['target'] = None
                    event['unbound_reason'] = 'no_active'
            data[state['query_name']]['task_bindings']['all_targets_bound'] = False
        self.mutated(state, 'optional-point-absent', optional_point, preserve_edge=True)
        def transaction_state(data):
            data[state['resource_name']]['transactions'][0]['after']['assigned'] += 1
        self.mutated(state, 'transaction-conservation', transaction_state)
        def independent_counter(data):
            data[state['resource_name']]['end']['counters']['acquire_returns'] += 1
        self.mutated(state, 'independent-return-counter', independent_counter)
        def allocator(data):
            data[state['resource_name']]['end']['allocators']['acquire_return']['high_water'] = 0
        self.mutated(state, 'allocator-bound', allocator)
        def loss(data):
            data[state['resource_name']]['lost_events'] += 1
        self.mutated(state, 'loss-contradiction', loss)
        def missing_return(data):
            for transaction in data[state['resource_name']]['transactions']:
                transaction['events'] = [event for event in transaction['events']
                    if not (event['kind'] == 'acquire_returned'
                            and event['request_id'] == state['target_request'])]
        self.mutated(state, 'missing-target-return-endpoint', missing_return)
        def missing_envelope(data):
            resource = data[state['resource_name']]
            resource['transactions'] = [tx for tx in resource['transactions']
                if not any(e['kind'] == 'grant' and e['request_id'] == state['target_request']
                           for e in tx['events'])]
        self.mutated(state, 'whole-source-grant-envelope-omitted', missing_envelope)
        def unknown_field(data):
            data[state['resource_name']]['invented_authority'] = True
        self.mutated(state, 'strict-unknown-field', unknown_field)
        def overflow(data):
            data[state['resource_name']]['loss_overflowed'] = True
        self.mutated(state, 'native-overflow-flag', overflow)


if __name__ == '__main__':
    unittest.main(verbosity=2)
