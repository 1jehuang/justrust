#!/usr/bin/env python3
"""Real compiler-private nested join queries, opt in with JR_QUERY_RUSTC.

Builds only the callback fixture, never the compiler. Existing stage0-sysroot
private metadata plus the matching stage1 driver suffice. Evidence survives
under JR_QUERY_EVIDENCE_DIR. Missing private metadata is a failure, not a skip.
"""
import collections
import copy
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


def assert_binding_inventory(test, receipt, complete=None):
    """Check independent allocator expectations, never derive them from rows."""
    test.assertEqual(receipt['schema'], 4, 'requires rebuilt schema4 producer')
    bindings = receipt['task_bindings']
    test.assertEqual(bindings['schema'], 1)
    reads = receipt['query_reads']
    highs = [c['allocator_high_water'] for c in receipt['contexts']]
    highs += [reads[k] for k in ('request_high_water', 'read_high_water', 'result_high_water')]
    highs += [receipt['wait_allocator_high_water']]
    expected = None if (receipt['expected_contexts'] != len(receipt['contexts'])
                        or any(h is None or h < 1 for h in highs)) else (
        sum(h - 1 for h in highs[:-1]) + 2 * (highs[-1] - 1))
    if expected is not None and expected > 2**64 - 1:
        expected = None
    test.assertEqual(bindings['expected_points'], expected)
    rows = bindings['events']
    sources = [json.dumps(row['source'], sort_keys=True) for row in rows]
    test.assertEqual(len(sources), len(set(sources)))
    for row in rows:
        test.assertEqual(row['target'] is None, row['unbound_reason'] is not None, row)
    if complete is not None:
        test.assertEqual(bindings['records_complete'], complete, bindings)
    if bindings['records_complete']:
        test.assertEqual(len(rows), expected)
        test.assertEqual(bindings['lost_events'], 0)
        test.assertFalse(bindings['loss_overflowed'])
        expected_sources = [dict(kind='admitted', context_id=c['context_id'], job_id=job)
                            for c in receipt['contexts']
                            for job in range(1, c['allocator_high_water'])]
        for kind, high in (('request_started', reads['request_high_water']),
                           ('dep_read', reads['read_high_water']),
                           ('result_produced', reads['result_high_water']),
                           ('wait_registered', receipt['wait_allocator_high_water']),
                           ('wait_resumed', receipt['wait_allocator_high_water'])):
            field = {'request_started': 'request_id', 'dep_read': 'read_id',
                     'result_produced': 'result_id', 'wait_registered': 'wait_id',
                     'wait_resumed': 'wait_id'}[kind]
            expected_sources += [dict(kind=kind, **{field: value}) for value in range(1, high)]
        test.assertEqual(set(sources), {json.dumps(row, sort_keys=True)
                                        for row in expected_sources})
    return bindings


@unittest.skipUnless(os.environ.get('JR_QUERY_RUSTC'), 'requires JR_QUERY_RUSTC')
class NativeTaskBindingAcceptance(unittest.TestCase):
    def run_logged(self, command, label, env=None):
        result = subprocess.run([str(a) for a in command], env=env or self.env,
                                cwd=self.root, capture_output=True, text=True, timeout=300)
        (self.root / (label + '.json')).write_text(json.dumps({
            'command': [str(a) for a in command], 'exit_code': result.returncode,
            'stdout': result.stdout, 'stderr': result.stderr}, indent=2))
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        return result

    def setUp(self):
        base = os.environ.get('JR_QUERY_EVIDENCE_DIR') or os.environ.get('JCODE_SCRATCH_DIR')
        if base:
            Path(base).mkdir(parents=True, exist_ok=True)
        self.root = Path(tempfile.mkdtemp(prefix='native-task-binding-', dir=base))
        print(json.dumps({'evidence_dir': str(self.root), 'retained': True}), flush=True)
        self.env = os.environ.copy()
        self.env.pop('JUSTRUST_EXECUTION_GRAPH_DIR', None)
        self.rustc = Path(os.environ['JR_QUERY_RUSTC']).resolve(strict=True)
        self.justrust = os.environ.get('JR_QUERY_JUSTRUST') or shutil.which('justrust')
        self.assertTrue(self.justrust, 'native consumer required')
        self.stage = self.rustc.parent.parent
        host_build = self.stage.parent
        stage0 = host_build / 'stage0/bin/rustc'
        sysroot = host_build / 'stage0-sysroot'
        drivers = list((self.stage / 'lib').glob('librustc_driver-*.so'))
        self.assertEqual(len(drivers), 1, 'require unique rebuilt driver')
        driver = drivers[0]
        metadata = list(sysroot.glob('lib/rustlib/*/lib/' + driver.with_suffix('.rmeta').name))
        self.assertEqual(len(metadata), 1, 'matching full private metadata unavailable')
        self.env['RUSTC_BOOTSTRAP'] = '1'
        self.env['LD_LIBRARY_PATH'] = os.pathsep.join([
            str(driver.parent), str(metadata[0].parent), str(sysroot / 'lib'),
            self.env.get('LD_LIBRARY_PATH', '')])
        (self.root / 'private-artifacts.json').write_text(json.dumps({
            str(path): {'sha256': hashlib.sha256(path.read_bytes()).hexdigest(),
                        'mtime_ns': path.stat().st_mtime_ns}
            for path in (stage0, driver, metadata[0])}, indent=2))
        source = Path(__file__).resolve().parent / 'fixtures/execution_graph_task_binding.rs'
        self.callback = self.root / 'native-callback'
        self.run_logged([stage0, '--sysroot', sysroot,
                         '-Ldependency=' + str(metadata[0].parent),
                         '-Ldependency=' + str(driver.parent),
                         '--extern', 'rustc_driver=' + str(metadata[0]),
                         '--extern', 'rustc_driver=' + str(driver), '-Cprefer-dynamic',
                         source, '-o', self.callback], 'callback-build')

    def assess(self, directory, label):
        result = self.run_logged([self.justrust, 'graph', directory, '--json'], label)
        assessments = json.loads(result.stdout)
        queries = [a for a in assessments if a['producer'] == 'rustc-query-admission']
        self.assertEqual(len(queries), 1, assessments)
        return queries[0]

    def test_nested_queries_continuations_and_negative_authority(self):
        source = self.root / 'input.rs'
        source.write_text('\n'.join(f'fn f{i}() -> u64 {{ {i} }}' for i in range(8)) +
                          '\nfn main() { println!("{}", ' +
                          '+'.join(f'f{i}()' for i in range(8)) + '); }\n')
        artifact = self.root / 'input-bin'
        sink = self.root / 'capture'
        command = [self.callback, '--sysroot', self.stage, '-Zthreads=4',
                   '--crate-name=task_binding_input', source, '-o', artifact]
        control = self.run_logged(command, 'control-build')
        baseline_output = self.run_logged([artifact], 'control-execute').stdout
        self.assertEqual(baseline_output, '28\n')
        captured = self.run_logged(command + ['-Zjr-query-graph=' + str(sink)], 'capture-build')
        self.assertEqual(captured.stdout + captured.stderr, control.stdout + control.stderr)
        self.assertEqual(self.run_logged([artifact], 'execute').stdout, baseline_output)
        files = {p: json.loads(p.read_text()) for p in sink.glob('*.json')}
        queries = [(p, r) for p, r in files.items() if r['producer'] == 'rustc-query-admission']
        self.assertEqual(len(queries), 1)
        query_path, query = queries[0]
        bindings = assert_binding_inventory(self, query, complete=True)
        ref = bindings['pool']
        self.assertIsNotNone(ref)
        pools = [(p, r) for p, r in files.items() if r['producer'] == 'rustc-thread-pool-join'
                 and r['capture_id'] == ref['capture_id']]
        self.assertEqual(len(pools), 1)
        pool_path, pool = pools[0]
        self.assertEqual(pool['schema'], 2)
        self.assertEqual(pool['query_capture_id'], query['capture_id'])
        self.assertEqual(pool['observer'], ref['observer'])
        forks = [e for e in pool['events'] if e['kind'] == 'fork']
        children = {e['child'] for e in forks}
        continuations = {e['continuation'] for e in forks}
        self.assertTrue(any(e['parent'] in children for e in forks), 'no actual nested native fork')
        self.assertTrue(continuations.isdisjoint(children), 'continuation aliases branch')
        points = bindings['events']
        admitted = {e['target']['task'] for e in points
                    if e['target'] is not None and e['source']['kind'] == 'admitted'}
        self.assertTrue(admitted & children, 'no actual query admission inside child')
        self.assertTrue(admitted & continuations, 'no actual distinct continuation query')
        self.assertTrue(any(e['target'] is None for e in points),
                        'outside/pre-attachment queries must remain explicitly unbound')
        read_events = query['query_reads']['events']
        cached_requests = {e['request_id'] for e in read_events if e['kind'] == 'cache_hit'}
        unbound_requests = {e['source']['request_id'] for e in points
                            if e['source']['kind'] == 'request_started' and e['target'] is None}
        self.assertTrue(cached_requests & unbound_requests,
                        'cached request outside native joins must remain unbound')
        opaque_reads = {e['read_id'] for e in read_events if e['kind'] == 'dep_read'
                        and e['outcome'] != 'tracked_new'}
        unbound_reads = {e['source']['read_id'] for e in points
                         if e['source']['kind'] == 'dep_read' and e['target'] is None}
        self.assertTrue(opaque_reads & unbound_reads,
                        'opaque read outside native joins must remain unbound')
        baseline = self.assess(sink, 'assessment')
        self.assertTrue(baseline['task_bindings']['graph_binding_valid'], baseline)
        self.assertEqual(baseline['task_bindings']['graph_binding_complete'],
                         bindings['all_targets_bound'], baseline)
        starts = {e['task']: e['at_ns'] for e in pool['events'] if e['kind'] == 'start'}
        finishes = {e['task']: e['at_ns'] for e in pool['events'] if e['kind'] == 'finish'}
        for row in points:
            if row['target'] is not None:
                target = row['target']
                self.assertIn(target['task'], starts)
                self.assertIn(target['task'], finishes)
                self.assertLessEqual(starts[target['task']], target['at_pool_ns'])
                self.assertLessEqual(target['at_pool_ns'], finishes[target['task']])
        self.assertTrue(baseline['task_bindings']['records_complete'], baseline)
        for label in ('missing-pool', 'remapped-task', 'duplicate-pool', 'foreign-query',
                      'stale-observer', 'same-task-foreign-capture', 'outside-interval'):
            altered = self.root / label
            shutil.copytree(sink, altered)
            q = copy.deepcopy(query)
            p = copy.deepcopy(pool)
            if label == 'missing-pool':
                (altered / pool_path.name).unlink()
            elif label == 'duplicate-pool':
                (altered / 'duplicate-pool.json').write_text(json.dumps(p))
            elif label == 'foreign-query':
                p['query_capture_id'] = 'foreign-query'
            elif label == 'same-task-foreign-capture':
                p['capture_id'] = 'foreign-pool'
            elif label == 'stale-observer':
                q['task_bindings']['pool']['observer'] += 1
            else:
                row = next(e for e in q['task_bindings']['events'] if e['target'] is not None)
                if label == 'remapped-task':
                    row['target']['task'] = pool['task_allocator_high_water'] + 1
                else:
                    row['target']['at_pool_ns'] = max(e['at_ns'] for e in pool['events']) + 1
            if label != 'missing-pool':
                (altered / pool_path.name).write_text(json.dumps(p))
            (altered / query_path.name).write_text(json.dumps(q))
            assessment = self.assess(altered, label + '-assessment')
            self.assertFalse(assessment['task_bindings']['graph_binding_valid'], assessment)
            self.assertFalse(assessment['task_bindings']['graph_binding_complete'], assessment)
            self.assertNotEqual(assessment['task_bindings']['issues'],
                                baseline['task_bindings']['issues'], (label, assessment))
            for key in ('structural_complete', 'wait_structural_complete', 'waits', 'nodes', 'query_reads'):
                self.assertEqual(assessment[key], baseline[key], (label, key, assessment))
        waits = collections.Counter(e['source']['kind'] for e in points
                                    if e['source']['kind'].startswith('wait_'))
        # This bounded nested fixture does not guarantee contention. The separate
        # native wait gate asserts real registered/resumed points under contention.
        print(json.dumps({'artifact_sha256': hashlib.sha256(artifact.read_bytes()).hexdigest(),
                          'actual_points': len(points), 'expected_points': bindings['expected_points'],
                          'bound_admission_tasks': sorted(admitted), 'wait_points': dict(waits),
                          'wait_exercise': 'observed' if waits else 'blocked: bounded fixture has no actual wait; run wait_native contention gate'}), flush=True)


if __name__ == '__main__':
    unittest.main(verbosity=2)
