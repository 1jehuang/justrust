#!/usr/bin/env python3
"""Opt-in real locked itoa rlib capture, using an already accepted private compiler.

JR_FRONTEND_RUSTC pins the accepted runtime compiler. JR_FRONTEND_METADATA_SYSROOT
and JR_FRONTEND_STAGE0 pin matching private metadata and fixture compiler. Builds
only fixtures and the real registry crate, never bootstraps rustc. Local execution
is required for this diagnostic compiler/private callback authority. All evidence
is retained. No elapsed interval is interpreted as CPU work or critical path.
"""
import collections
import fcntl
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import tomllib
import unittest

from test_execution_graph_task_binding_native import assert_binding_inventory


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def fixture_controls(sink, *, large=False, timing=False, dispatch=False, query_cpu=False,
                     execution_flow=False):
    query_cpu = query_cpu or execution_flow
    text = str(sink or '-') + '\n' + ('1000000' if large or timing or dispatch or query_cpu else 'default') + '\n'
    if timing or dispatch or query_cpu:
        text += ('timing' if timing or query_cpu else 'off') + '\n'
    if dispatch or query_cpu:
        text += ('dispatch' if dispatch else 'off') + '\n'
    if query_cpu:
        text += 'query_cpu\n'
    if execution_flow:
        text += 'execution_flow\n'
    return text


@unittest.skipUnless(os.environ.get('JR_FRONTEND_RUSTC'), 'requires accepted JR_FRONTEND_RUSTC')
class FrontendMappingNative(unittest.TestCase):
    def logged(self, command, label, control=None, expected=0):
        command = [str(value) for value in command]
        reserved = None
        if control is not None:
            read_fd, write_fd = os.pipe2(os.O_CLOEXEC)
            try:
                reserved = fcntl.fcntl(read_fd, fcntl.F_DUPFD_CLOEXEC, 198)
                if reserved != 198:
                    os.close(reserved)
                    reserved = None
                    self.fail('foreign fd198 occupied, refusing overwrite')
                encoded = control.encode('utf-8')
                self.assertLess(len(encoded), 4096, 'bounded fixture control packet')
                self.assertEqual(os.write(write_fd, encoded), len(encoded))
            except BaseException:
                if reserved is not None:
                    os.close(reserved)
                    reserved = None
                raise
            finally:
                os.close(read_fd)
                os.close(write_fd)
        try:
            result = subprocess.run(command, cwd=self.root, env=self.env,
                                    pass_fds=() if reserved is None else (reserved,),
                                    capture_output=True, text=True, timeout=300)
        finally:
            if reserved is not None:
                os.close(reserved)
        (self.root / (label + '.json')).write_text(json.dumps({
            'command': command, 'exit_code': result.returncode,
            'control_sha256': None if control is None else hashlib.sha256(control.encode()).hexdigest(),
            'environment_sha256': hashlib.sha256(json.dumps(sorted(self.env.items())).encode()).hexdigest(),
            'stdout': result.stdout, 'stderr': result.stderr}, indent=2))
        if expected is not None:
            self.assertEqual(result.returncode, expected, result.stdout + result.stderr)
        return result

    def setUp(self):
        repo = Path(__file__).resolve().parents[1]
        base = os.environ.get('JR_FRONTEND_EVIDENCE_DIR') or os.environ.get('JCODE_SCRATCH_DIR')
        self.assertTrue(base, 'explicit retained scratch directory required')
        Path(base).mkdir(parents=True, exist_ok=True)
        self.root = Path(tempfile.mkdtemp(prefix='frontend-real-itoa-', dir=base))
        print(json.dumps({'evidence_dir': str(self.root), 'retained': True}), flush=True)
        self.rustc = Path(os.environ['JR_FRONTEND_RUSTC']).resolve(strict=True)
        self.stage = self.rustc.parent.parent
        self.sysroot = Path(os.environ.get('JR_FRONTEND_RUN_SYSROOT', self.stage)).resolve(strict=True)
        self.stage0 = Path(os.environ['JR_FRONTEND_STAGE0']).resolve(strict=True)
        self.metadata = Path(os.environ['JR_FRONTEND_METADATA_SYSROOT']).resolve(strict=True)
        self.client = os.environ.get('JR_FRONTEND_JUSTRUST') or shutil.which('justrust')
        self.assertTrue(self.client, 'actual public graph consumer required')
        client_source = Path(self.client).resolve(strict=True)
        self.client = self.root / 'justrust-consumer'
        shutil.copy2(client_source, self.client)
        drivers = list((self.stage / 'lib').glob('librustc_driver-*.so'))
        self.assertEqual(len(drivers), 1, 'unique accepted runtime driver required')
        driver = drivers[0]
        metas = list(self.metadata.glob('lib/rustlib/*/lib/' + driver.with_suffix('.rmeta').name))
        self.assertEqual(len(metas), 1, 'matching full private metadata required')
        meta = metas[0]
        lock = tomllib.loads((repo / 'Cargo.lock').read_text())
        versions = [p['version'] for p in lock['package'] if p['name'] == 'itoa']
        self.assertEqual(len(versions), 1, 'one locked itoa version required')
        cargo_home = Path(os.environ.get('CARGO_HOME', Path.home() / '.cargo'))
        sources = list((cargo_home / 'registry/src').glob('*/itoa-' + versions[0]))
        configured = os.environ.get('JR_FRONTEND_ITOA_SOURCE')
        self.source_root = Path(configured).resolve(strict=True) if configured else (
            sources[0] if len(sources) == 1 else None)
        self.assertIsNotNone(self.source_root, 'locked registry crate source required')
        manifest = tomllib.loads((self.source_root / 'Cargo.toml').read_text())
        self.assertEqual(manifest['package']['version'], versions[0])
        self.assertEqual(manifest['package']['name'], 'itoa')
        self.assertFalse(manifest.get('features', {}).get('default', []),
                         'default feature cfg changed: update normal compile explicitly')
        self.assertFalse(manifest['package'].get('build', False), 'unexpected build script')
        self.source = self.source_root / manifest['lib']['path']
        self.edition = manifest['package']['edition']
        self.source_hashes = {str(p.relative_to(self.source_root)): digest(p)
                              for p in sorted((self.source_root / 'src').rglob('*.rs'))}
        self.env = os.environ.copy()
        for key in ('JUSTRUST_EXECUTION_GRAPH_DIR', 'RUSTC_WRAPPER', 'RUSTC_WORKSPACE_WRAPPER',
                    'RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS', 'CARGO_MAKEFLAGS', 'MAKEFLAGS', 'MFLAGS'):
            self.env.pop(key, None)
        self.env['RUSTC_BOOTSTRAP'] = '1'
        self.env['LD_LIBRARY_PATH'] = os.pathsep.join([
            str(driver.parent), str(meta.parent), str(self.metadata / 'lib'),
            self.env.get('LD_LIBRARY_PATH', '')])
        fixture = repo / 'tests/fixtures/frontend_mapping_driver.rs'
        self.analyzer = repo / 'bench/frontend_mapping.py'
        self.analyzer_sha256 = digest(self.analyzer)
        shutil.copyfile(fixture, self.root / fixture.name)
        (self.root / 'provenance.json').write_text(json.dumps({
            'crate': 'itoa', 'version': versions[0], 'edition': self.edition,
            'source_root': str(self.source_root), 'source_hashes': self.source_hashes,
            'default_features': [], 'local_reason': 'accepted private compiler diagnostic',
            'artifacts': {str(p): digest(p) for p in
                          (self.rustc, self.stage0, driver, meta, fixture, self.analyzer,
                           Path(__file__), repo / 'Cargo.lock',
                           self.client)}
        }, indent=2))
        self.callback = self.root / 'frontend-driver'
        build_args = [self.stage0, '--edition=2024', '--sysroot', self.metadata,
                     '-Ldependency=' + str(meta.parent), '-Ldependency=' + str(driver.parent),
                     '--extern', 'rustc_driver=' + str(meta),
                     '--extern', 'rustc_driver=' + str(driver), '-Cprefer-dynamic',
                     fixture, '-o', self.callback]
        if self._testMethodName == 'test_real_itoa_large_cold_mapping':
            build_args.append('--cfg=frontend_large_capacity')
        if self._testMethodName == 'test_real_itoa_timing_cold_and_incremental':
            build_args.extend(['--cfg=frontend_large_capacity', '--cfg=frontend_timing_api'])
        if self._testMethodName == 'test_real_itoa_dispatch_resolutions':
            build_args.extend(['--cfg=frontend_large_capacity', '--cfg=frontend_timing_api',
                               '--cfg=frontend_dispatch_api'])
        if self._testMethodName == 'test_real_itoa_query_cpu_ownership':
            build_args.extend(['--cfg=frontend_large_capacity', '--cfg=frontend_timing_api',
                               '--cfg=frontend_dispatch_api', '--cfg=frontend_query_cpu_api'])
        if self._testMethodName == 'test_real_itoa_execution_flow':
            build_args.extend(['--cfg=frontend_large_capacity', '--cfg=frontend_timing_api',
                               '--cfg=frontend_dispatch_api', '--cfg=frontend_query_cpu_api',
                               '--cfg=frontend_execution_flow_api'])
        if os.environ.get('JR_FRONTEND_SCOPE_API'):
            build_args.append('--cfg=frontend_scope_api')
        self.logged(build_args, 'driver-build')

    def compile(self, label, threads, output, incremental=None, capture=False, large=False,
                timing=False, dispatch=False, query_cpu=False, execution_flow=False):
        query_cpu = query_cpu or execution_flow
        timing = timing or query_cpu
        sink = self.root / (label + '-capture') if capture else None
        args = ['--sysroot', self.sysroot, '--crate-name=itoa', '--crate-type=rlib',
                '--edition=' + self.edition, '--emit=link', '-Copt-level=0', '-Cdebuginfo=0',
                '-Zthreads=' + str(threads), '--check-cfg=cfg(feature, values("no-panic"))',
                self.source, '-o', output]
        if incremental is not None:
            args.append('-Cincremental=' + str(incremental))
        control = fixture_controls(sink, large=large, timing=timing, dispatch=dispatch,
                                   query_cpu=query_cpu, execution_flow=execution_flow)
        result = self.logged([self.callback, *args], label, control=control)
        self.assertTrue(output.is_file(), 'real link-emitted rlib required')
        self.assertGreater(output.stat().st_size, 0)
        artifact = digest(output)
        shutil.copyfile(output, self.root / (label + '.rlib'))
        members_result = self.logged(['ar', 't', output], label + '-members')
        members = members_result.stdout.splitlines()
        self.assertEqual(len(members), len(set(members)), 'duplicate archive members')
        payloads = []
        for member in members:
            payload = subprocess.run(['ar', 'p', str(output), member], cwd=self.root,
                                     env=self.env, capture_output=True, timeout=30)
            self.assertEqual(payload.returncode, 0, payload.stderr.decode(errors='replace'))
            payloads.append({'name': member, 'sha256': hashlib.sha256(payload.stdout).hexdigest(),
                             'bytes': len(payload.stdout)})
        (self.root / (label + '-payloads.json')).write_text(json.dumps(payloads, indent=2))
        # Owned incremental linker metadata contains randomized object basenames.
        # Keep its raw hash, but scope payload neutrality to rmeta and object bytes.
        payload_hashes = sorted(p['sha256'] for p in payloads if p['name'] != 'lib.rmeta-link')
        consumer_source = self.root / 'consumer.rs'
        if not consumer_source.exists():
            consumer_source.write_text('fn main() { let mut b = itoa::Buffer::new(); '
                                      'for x in [i128::MIN, -1, 0, 1, i128::MAX] { '
                                      'assert_eq!(b.format(x), x.to_string()); } '
                                      'for x in [0u128, 1, u128::MAX] { '
                                      'assert_eq!(b.format(x), x.to_string()); } '
                                      'println!("itoa-boundaries-ok"); }\n')
        executable = self.root / 'consumer-bin'
        self.logged([self.rustc, '--sysroot', self.sysroot, '--edition=2021',
                     '--crate-name=itoa_consumer', '--extern', 'itoa=' + str(output),
                     consumer_source, '-o', executable], label + '-consumer-build')
        executed = self.logged([executable], label + '-consumer-run')
        self.assertEqual(executed.stdout, 'itoa-boundaries-ok\n')
        executable_hash = digest(executable)
        shutil.copyfile(executable, self.root / (label + '-consumer-bin'))
        if sink is None:
            return artifact, None, args, result.stderr, payload_hashes, executable_hash
        graph_command = [self.client, 'graph', sink, '--json']
        if execution_flow:
            rejected = self.logged(graph_command + ['--large-capture', '--query-cpu-ownership'],
                                   label + '-implicit-execution-flow-graph', expected=None)
            self.assertNotEqual(rejected.returncode, 0, 'pool5 requires explicit execution flow analysis')
            graph_command.append('--execution-flow-dag')
        if query_cpu:
            if not execution_flow:
                rejected = self.logged(graph_command + ['--large-capture'],
                                       label + '-implicit-query-cpu-graph', expected=None)
                self.assertNotEqual(rejected.returncode, 0, 'pool4 requires explicit query CPU analysis')
            graph_command.append('--query-cpu-ownership')
        if dispatch and not query_cpu:
            rejected = self.logged(graph_command, label + '-implicit-dispatch-graph', expected=None)
            self.assertNotEqual(rejected.returncode, 0, 'query6 requires explicit dispatch analysis')
        if dispatch:
            graph_command.append('--dispatch-result-provenance')
        if large:
            rejected = self.logged(graph_command, label + '-default-graph', expected=None)
            self.assertNotEqual(rejected.returncode, 0, 'oversized capture default guard must reject')
            graph_command.append('--large-capture')
        if (timing or dispatch) and not large:
            graph_command.append('--large-capture')
        graph = self.logged(graph_command, label + '-graph')
        assessments = json.loads(graph.stdout)
        (self.root / (label + '-assessment.json')).write_text(json.dumps(assessments, indent=2))
        queries = []
        pools = []
        for path in sorted(sink.glob('*.json')):
            row = json.loads(path.read_text())
            if row.get('producer') == 'rustc-query-admission':
                queries.append(row)
            elif row.get('producer') == 'rustc-thread-pool-join':
                pools.append(row)
        self.assertEqual(len(queries), 1, 'one actual native query root expected')
        query = queries[0]
        if dispatch:
            self.assertEqual(query['schema'], 6)
            bindings = self.assert_timed_point_inventory(query, schema=6)
            if not timing:
                self.assertIsNone(query['execution_clock'])
                for pool in pools:
                    self.assertEqual(pool['schema'], 2)
                    self.assertNotIn('timing', pool)
        elif timing:
            self.assertEqual(query['schema'], 5)
            bindings = self.assert_timed_point_inventory(query)
        else:
            bindings = assert_binding_inventory(self, query)
            self.assertEqual(query['schema'], 4)
            self.assertNotIn('execution_clock', query, 'default query4 bytes gained timing field')
            for pool in pools:
                self.assertEqual(pool['schema'], 2)
                self.assertNotIn('timing', pool, 'default pool2 bytes gained timing field')
        reads = query['query_reads']
        summary = {
            'label': label, 'threads': threads, 'artifact_sha256': artifact,
            'contexts': len(query['contexts']), 'expected_contexts': query['expected_contexts'],
            'admissions': sum(c['allocator_high_water'] - 1 for c in query['contexts']),
            'query_events': len(query['events']), 'query_lost_events': query['lost_events'],
            'waits': query['wait_allocator_high_water'] - 1,
            'read_records': len(reads['events']),
            'read_high_water': reads['read_high_water'],
            'request_high_water': reads['request_high_water'],
            'result_high_water': reads['result_high_water'],
            'read_lost_events': reads['lost_events'],
            'binding_expected': bindings['expected_points'],
            'binding_records': len(bindings['events']),
            'binding_lost_events': bindings['lost_events'],
            'binding_records_complete': bindings['records_complete'],
            'bound_points': sum(e['target'] is not None for e in bindings['events']),
            'unbound_reasons': dict(collections.Counter(e['unbound_reason'] for e in
                                   bindings['events'] if e['target'] is None)),
            'pool_receipts': len(pools),
            'pool_events': sum(len(p['events']) for p in pools),
            'pool_root_events': sum(sum(e['kind'] == 'root' for e in p['events']) for p in pools),
            'pool_dropped_events': sum(p['dropped_events'] for p in pools),
            'public': assessments,
        }
        if dispatch:
            summary['dispatch'] = self.assert_dispatch_receipts(query, assessments, sink, label,
                                                               query_cpu=query_cpu)
        if timing:
            summary['timing'] = self.assert_timed_receipts(query, pools, assessments, sink, label,
                                                        large=large, query_cpu=query_cpu,
                                                        execution_flow=execution_flow)
        if query_cpu:
            summary['query_cpu'] = self.assert_query_cpu_receipts(query, pools, assessments)
        if execution_flow:
            summary['execution_flow'] = self.assert_execution_flow_receipts(query, pools, assessments)
        (self.root / (label + '-inventory.json')).write_text(json.dumps(summary, indent=2))
        print(json.dumps({k: v for k, v in summary.items() if k != 'public'}, sort_keys=True), flush=True)
        query_public = [a for a in assessments if a['producer'] == 'rustc-query-admission']
        self.assertEqual(len(query_public), 1)
        public = query_public[0]
        self.assertEqual(query['lost_events'], 0, 'root admissions unexpectedly lost')
        self.assertEqual(query['expected_contexts'], len(query['contexts']))
        self.assertEqual(len(query['events']),
                         2 * sum(c['allocator_high_water'] - 1 for c in query['contexts']))
        self.assertTrue(public['wait_structural_complete'])
        if reads['lost_events'] or bindings['lost_events']:
            self.assertFalse(public['structural_complete'], 'loss must refuse completeness')
        else:
            self.assertTrue(public['structural_complete'])
            self.assertTrue(public['task_bindings']['source_endpoints_complete'])
            self.assertEqual(public['query_reads']['lost_events'], 0)
            if threads > 1:
                self.assertGreater(summary['bound_points'], 0, 'real parallel task binding required')
                self.assertTrue(public['task_bindings']['graph_binding_valid'],
                                public['task_bindings']['issues'])
        for assessment in assessments:
            self.assertFalse(assessment['full_pipeline_complete'])
            self.assertIsNone(assessment['cpu_work_ns'])
            self.assertIsNone(assessment['execution_span_ns'])
        return artifact, summary, args, result.stderr, payload_hashes, executable_hash

    def assert_timed_point_inventory(self, query, schema=5):
        """Check allocator contracts without rewriting actual query5/query6 headers."""
        self.assertEqual(query['schema'], schema)
        bindings = query['task_bindings']
        reads = query['query_reads']
        contexts = {c['context_id']: c['allocator_high_water'] for c in query['contexts']}
        self.assertEqual(query['expected_contexts'], len(contexts))
        highs = list(contexts.values()) + [reads[k] for k in (
            'request_high_water', 'read_high_water', 'result_high_water')]
        highs.append(query['wait_allocator_high_water'])
        self.assertTrue(all(isinstance(h, int) and h >= 1 for h in highs))
        expected = sum(h - 1 for h in highs[:-1]) + 2 * (highs[-1] - 1)
        self.assertEqual(bindings['expected_points'], expected)
        self.assertTrue(bindings['records_complete'])
        self.assertEqual(bindings['lost_events'], 0)
        self.assertFalse(bindings['loss_overflowed'])
        self.assertEqual(len(bindings['events']), expected)
        limits = {
            'request_started': ('request_id', reads['request_high_water']),
            'dep_read': ('read_id', reads['read_high_water']),
            'result_produced': ('result_id', reads['result_high_water']),
            'wait_registered': ('wait_id', query['wait_allocator_high_water']),
            'wait_resumed': ('wait_id', query['wait_allocator_high_water']),
        }
        seen = set()
        for row in bindings['events']:
            source = row['source']
            identity = tuple(sorted(source.items()))
            self.assertNotIn(identity, seen, 'duplicate actual timing source')
            seen.add(identity)
            if source['kind'] == 'admitted':
                self.assertEqual(set(source), {'kind', 'context_id', 'job_id'})
                self.assertIn(source['context_id'], contexts)
                self.assertTrue(1 <= source['job_id'] < contexts[source['context_id']])
            else:
                self.assertIn(source['kind'], limits)
                field, high = limits[source['kind']]
                self.assertEqual(set(source), {'kind', field})
                self.assertTrue(1 <= source[field] < high)
            self.assertEqual(row['target'] is None, row['unbound_reason'] is not None)
        return bindings

    def assert_timed_receipts(self, query, pools, assessments, sink, label, *, large,
                              query_cpu=False, execution_flow=False):
        self.assertEqual(len(pools), 1, 'one actual timed native pool required')
        pool = pools[0]
        self.assertEqual(pool['schema'], 5 if execution_flow else (4 if query_cpu else 3))
        clock = query['execution_clock']
        self.assertIsNotNone(clock, 'actual attached query5 clock required')
        self.assertEqual(set(clock), {'query_capture_id', 'pool_capture_id', 'pool_observer',
                                     'pool_epoch_in_query_ns'})
        self.assertEqual(clock, {
            'query_capture_id': query['capture_id'], 'pool_capture_id': pool['capture_id'],
            'pool_observer': pool['observer'], 'pool_epoch_in_query_ns': 0})
        self.assertEqual(pool['query_capture_id'], query['capture_id'])
        self.assertEqual(query['task_bindings']['pool'], {
            'capture_id': pool['capture_id'], 'observer': pool['observer']})
        timing = pool['timing']
        self.assertEqual(timing['clock_bridge'], clock)
        self.assertEqual(timing['schema'], 2 if query_cpu else 1)
        self.assertEqual(timing['scope'], 'observed-exclusive-thread-segments')
        self.assertEqual(timing['cpu_clock'], 'linux-thread-cpu')
        self.assertTrue(timing['sealed'])
        self.assertFalse(timing['identity_exhausted'])
        self.assertFalse(timing['loss_overflowed'])
        self.assertEqual(timing['lost_segments'], 0, 'actual timing cap must not hide source work')
        self.assertEqual(timing['active_segments'], 0)
        self.assertEqual(timing['segment_allocator_baseline'], 1)
        self.assertEqual(timing['expected_segments'], timing['segment_allocator_high_water'] - 1)
        segments = timing['segments']
        self.assertEqual(len(segments), timing['expected_segments'])
        self.assertGreater(len(segments), 0)
        ids = set()
        per_thread = collections.defaultdict(list)
        task_cpu = opaque_cpu = query_execution_cpu = measured = failed = 0
        envelope_uncertain = False
        for segment in segments:
            sid = segment['segment_id']
            self.assertNotIn(sid, ids)
            ids.add(sid)
            self.assertTrue(1 <= sid < timing['segment_allocator_high_water'])
            self.assertTrue(1 <= segment['thread_id'] < timing['thread_allocator_high_water'])
            if segment['owner_kind'] == 'query_execution':
                self.assertTrue(query_cpu, 'legacy timing gained query ownership')
                self.assertIsNotNone(segment['query_context'])
                self.assertIsNotNone(segment['query_job'])
            else:
                self.assertEqual(segment['task'] is None, segment['owner_kind'] == 'opaque')
            per_thread[segment['thread_id']].append(segment)
            if segment['status'] == 'measured':
                self.assertIsNotNone(segment['cpu_start_ns'])
                self.assertIsNotNone(segment['cpu_end_ns'])
                self.assertEqual(segment['cpu_ns'], segment['cpu_end_ns'] - segment['cpu_start_ns'])
                self.assertGreaterEqual(segment['cpu_ns'], 0)
                if segment['cpu_ns'] > segment['end_ns'] - segment['start_ns']:
                    # Distinct clock domains lack an exact rate/error bound.
                    # Retain authentic raw bytes but require unknown aggregates.
                    envelope_uncertain = True
                if segment['owner_kind'] == 'task':
                    task_cpu += segment['cpu_ns']
                elif segment['owner_kind'] == 'query_execution':
                    query_execution_cpu += segment['cpu_ns']
                else:
                    opaque_cpu += segment['cpu_ns']
                measured += 1
            else:
                self.assertIn(segment['status'], {'clock_failed', 'clock_regressed', 'wall_overflow'})
                self.assertIsNone(segment['cpu_ns'], 'failed clock never fabricates zero CPU')
                failed += 1
        regressed = False
        per_task = collections.defaultdict(list)
        for segment in segments:
            if segment['task'] is not None:
                per_task[segment['task']].append(segment)
        for rows in per_task.values():
            rows.sort(key=lambda s: (s['start_ns'] is None, s['start_ns'] or 0, s['segment_id']))
            end = None
            for segment in rows:
                if segment['start_ns'] is not None and segment['end_ns'] is not None:
                    if end is not None:
                        self.assertLessEqual(end, segment['start_ns'], 'same-task cross-thread overlap')
                    end = segment['end_ns']
        for rows in per_thread.values():
            thread_regressed = any(s['status'] == 'clock_regressed' for s in rows)
            regressed |= thread_regressed
            rows.sort(key=lambda s: (s['start_ns'] is None, s['start_ns'] or 0, s['segment_id']))
            wall_end = cpu_end = None
            for segment in rows:
                if segment['start_ns'] is not None and segment['end_ns'] is not None:
                    if wall_end is not None:
                        self.assertLessEqual(wall_end, segment['start_ns'], 'exclusive wall overlap')
                    wall_end = segment['end_ns']
                if segment['status'] == 'measured' and not thread_regressed:
                    if cpu_end is not None:
                        self.assertLessEqual(cpu_end, segment['cpu_start_ns'], 'exclusive CPU overlap')
                    cpu_end = segment['cpu_end_ns']
        if not failed:
            self.assertGreater(task_cpu + opaque_cpu + query_execution_cpu, 0,
                               'actual busy compiler must have scoped CPU')
        query_public = next(a for a in assessments if a['producer'] == 'rustc-query-admission')
        pool_public = next(a for a in assessments if a['producer'] == 'rustc-thread-pool-join')
        self.assertTrue(query_public['clock_alignment']['aligned'],
                        query_public['clock_alignment']['issues'])
        cpu = pool_public['observed_cpu']
        self.assertEqual(cpu['clock_bridge'], clock)
        aggregates_unknown = envelope_uncertain or regressed
        if aggregates_unknown:
            self.assertIsNone(cpu['observed_cpu_ns'])
            self.assertIsNone(cpu['task_cpu_ns'])
            self.assertIsNone(cpu['opaque_cpu_ns'])
            if query_cpu:
                self.assertIsNone(cpu['query_execution_cpu_ns'])
            self.assertFalse(cpu['records_complete'])
            self.assertTrue(cpu['issues'], 'unknown quantities need an explicit cause')
        else:
            self.assertEqual(cpu['observed_cpu_ns'], task_cpu + opaque_cpu + query_execution_cpu)
            self.assertEqual(cpu['task_cpu_ns'], task_cpu)
            self.assertEqual(cpu['opaque_cpu_ns'], opaque_cpu)
            if query_cpu:
                self.assertEqual(cpu['query_execution_cpu_ns'], query_execution_cpu)
        self.assertEqual(cpu['measured_segments'], measured)
        self.assertEqual(cpu['failed_segments'], failed)
        self.assertEqual(cpu['observed_threads'], len(per_thread))
        if failed or aggregates_unknown or timing['clock_failures']:
            self.assertFalse(cpu['records_complete'])
        else:
            self.assertTrue(cpu['records_complete'])
        # Mutate only query endorsement. Original pool CPU/clock bytes stay unchanged.
        negative = self.root / (label + '-unilateral-clock')
        negative.mkdir(mode=0o700)
        for path in sink.glob('*.json'):
            raw = json.loads(path.read_text())
            if raw['producer'] == 'rustc-query-admission':
                raw['execution_clock']['pool_observer'] += 1
                (negative / path.name).write_text(json.dumps(raw))
            else:
                shutil.copyfile(path, negative / path.name)
        replay_command = [self.client, 'graph', negative, '--json', '--large-capture']
        if execution_flow:
            replay_command.append('--execution-flow-dag')
        if query_cpu:
            replay_command.append('--query-cpu-ownership')
        if query['schema'] == 6:
            replay_command.append('--dispatch-result-provenance')
        rejected = self.logged(replay_command,
                               label + '-unilateral-clock-graph', expected=None)
        if rejected.returncode == 0:
            changed = json.loads(rejected.stdout)
            changed_query = next(a for a in changed if a['producer'] == 'rustc-query-admission')
            self.assertFalse(changed_query['clock_alignment']['aligned'],
                             'unilateral clock declaration acquired reciprocal authority')
            changed_pool = next(a for a in changed if a['producer'] == 'rustc-thread-pool-join')
            if not query_cpu:
                self.assertEqual(changed_pool['observed_cpu'], cpu,
                                 'foreign query clock erased independent pool CPU evidence')
            else:
                changed_cpu = changed_query['query_cpu_ownership']
                self.assertFalse(changed_cpu['records_complete'])
                self.assertIsNone(changed_cpu['query_execution_cpu_ns'])
                self.assertEqual(changed_cpu['queries'], [])
            self.assertEqual(changed_query['task_bindings'], query_public['task_bindings'],
                             'clock mismatch altered independent point authority')
        return {'scoped_observed_cpu_ns': cpu['observed_cpu_ns'], 'task_cpu_ns': cpu['task_cpu_ns'],
                'opaque_cpu_ns': cpu['opaque_cpu_ns'], 'segments': len(segments),
                'aggregate_unknown': aggregates_unknown, 'cpu_issues': cpu['issues'],
                'measured_segments': measured, 'failed_segments': failed,
                'observed_threads': len(per_thread), 'clock_aligned': True,
                'unilateral_clock_rejected_or_unaligned': True}

    def assert_query_cpu_receipts(self, query, pools, assessments):
        """Check every actual execution owner against original admitted lifetimes."""
        admitted, completed = {}, {}
        for event in query['events']:
            key = event['context_id'], event['job_id']
            target = admitted if event['kind'] == 'admitted' else completed
            self.assertNotIn(key, target)
            target[key] = event['at_ns']
        segments = pools[0]['timing']['segments']
        costs = collections.Counter()
        per_owner = collections.defaultdict(list)
        names = {}
        owned_segments = 0
        for segment in segments:
            self.assertIn('query_context', segment)
            self.assertIn('query_job', segment)
            self.assertIn('query_name', segment)
            context, job = segment['query_context'], segment['query_job']
            self.assertEqual(context is None, job is None)
            self.assertEqual(context is None, segment['query_name'] is None)
            self.assertEqual(context is not None, segment['owner_kind'] == 'query_execution')
            if context is None:
                continue
            owner = context, job
            name = segment['query_name']
            self.assertTrue(name.isascii() and name.isidentifier() and len(name) <= 128)
            self.assertEqual(names.setdefault(owner, name), name, 'query kind changed for actual job')
            self.assertIn(owner, admitted)
            self.assertIn(owner, completed)
            if segment['start_ns'] is not None and segment['end_ns'] is not None:
                self.assertLessEqual(admitted[owner], segment['start_ns'])
                self.assertLessEqual(segment['end_ns'], completed[owner])
                per_owner[owner].append((segment['start_ns'], segment['end_ns']))
            if segment['status'] == 'measured':
                costs[owner] += segment['cpu_ns']
                owned_segments += 1
        for intervals in per_owner.values():
            intervals.sort()
            for left, right in zip(intervals, intervals[1:]):
                self.assertLessEqual(left[1], right[0], 'same-query cross-thread overlap')
        public = next(a for a in assessments if a['producer'] == 'rustc-query-admission')
        cpu = public['query_cpu_ownership']
        pool_cpu = next(a for a in assessments
                        if a['producer'] == 'rustc-thread-pool-join')['observed_cpu']
        self.assertEqual(cpu['scope'], 'observed-query-execution-cpu')
        self.assertEqual(cpu['measured_segments'], owned_segments)
        # Honest unknowns are correct negative behavior, but cannot satisfy this
        # positive information-gain acceptance. Do not pass an all-unknown run.
        self.assertIsNotNone(pool_cpu['query_execution_cpu_ns'], pool_cpu['issues'])
        self.assertIsNotNone(cpu['query_execution_cpu_ns'], cpu['issues'])
        self.assertGreater(owned_segments, 0, 'real compiler must execute observed queries')
        self.assertGreater(cpu['query_execution_cpu_ns'], 0, 'actual query CPU required')
        expected = [dict(query_context=context, query_job=job,
                         query_name=names[(context, job)], cpu_ns=cost)
                    for (context, job), cost in costs.items()]
        expected.sort(key=lambda row: (-row['cpu_ns'], row['query_context'], row['query_job']))
        self.assertEqual(cpu['queries'], expected)
        self.assertEqual(cpu['query_execution_cpu_ns'], sum(costs.values()))
        self.assertEqual(cpu['query_execution_cpu_ns'], pool_cpu['query_execution_cpu_ns'])
        self.assertTrue(cpu['records_complete'], cpu['issues'])
        return dict(query_execution_cpu_ns=cpu['query_execution_cpu_ns'],
                    measured_segments=owned_segments, observed_query_owners=len(costs),
                    all_original_owners_checked=True, records_complete=cpu['records_complete'])

    def assert_execution_flow_receipts(self, query, pools, assessments):
        from bench.execution_flow_oracle import execution_flow_oracle
        pool, = pools
        self.assertEqual(pool['schema'], 5)
        expected = execution_flow_oracle(query, pool)
        public = next(a for a in assessments if a['producer'] == 'rustc-thread-pool-join')
        flow = public['execution_flow_dag']
        self.assertEqual(flow['scope'], 'observed-query-and-join-control-flow')
        self.assertTrue(flow['records_complete'], flow['issues'])
        self.assertEqual(flow['issues'], [])
        for key in ('nodes', 'edges', 'scopes', 'assigned_cpu_segments',
                    'known_cpu_work_ns', 'known_query_cpu_ns', 'known_task_cpu_ns',
                    'known_opaque_cpu_ns', 'longest_known_chain_cpu_ns'):
            self.assertEqual(flow[key], expected[key], key)
        self.assertEqual(flow['unassigned_cpu_segments'], 0)
        self.assertEqual(flow['missing_measurement_references'], 0)
        self.assertEqual(flow['unavailable_cpu_segments'], 0)
        self.assertGreater(flow['known_query_cpu_ns'], 0)
        self.assertGreater(flow['longest_known_chain_cpu_ns'], 0)
        self.assertFalse(flow['flow_complete'])
        self.assertFalse(flow['full_pipeline_complete'])
        for key in ('cpu_work_ns', 'execution_span_ns', 'parallelism'):
            self.assertIsNone(flow[key])
        nodes = {node['id']: node for node in pool['execution_flow']['nodes']}
        segments = {row['segment_id']: row for row in pool['timing']['segments']}
        edges = {(edge['from'], edge['to']) for edge in pool['execution_flow']['edges']}
        path = flow['longest_known_chain_path']
        steps = flow['longest_known_chain_steps']
        self.assertEqual(path, [step['node_id'] for step in steps])
        self.assertEqual(len(path), len(set(path)))
        self.assertTrue(all(pair in edges for pair in zip(path, path[1:])))
        self.assertEqual(sum(step['cpu_ns'] for step in steps), flow['longest_known_chain_cpu_ns'])
        for step in steps:
            node = nodes[step['node_id']]
            self.assertNotEqual(node['kind'], 'cut', 'unknown cut cannot bridge a known chain')
            self.assertEqual(step['kind'], node['kind'])
            self.assertEqual(step['at_ns'], node['at_ns'])
            self.assertEqual(step['segment_id'], node['segment_id'])
            if node['kind'] == 'cpu_segment':
                segment = segments[node['segment_id']]
                self.assertEqual(step['cpu_ns'], segment['cpu_ns'])
                for key in ('query_context', 'query_job', 'query_name'):
                    self.assertEqual(step[key], segment[key])
            else:
                self.assertEqual(step['cpu_ns'], 0)
        return {key: flow[key] for key in (
            'scope', 'records_complete', 'nodes', 'edges', 'scopes', 'assigned_cpu_segments',
            'known_cpu_work_ns', 'known_query_cpu_ns', 'known_task_cpu_ns',
            'known_opaque_cpu_ns', 'longest_known_chain_cpu_ns', 'cut_nodes')}

    def assert_execution_flow_analyzer(self, evidence):
        label = evidence['label']
        sink = self.root / (label + '-capture')
        original = {p.name: digest(p) for p in sink.glob('*.json')}
        output = self.root / (label + '-flow-summary.json')
        graph = self.root / (label + '-flow-public.json')
        command = ['python3', self.analyzer, '--capture', sink, '--justrust', self.client,
                   '--large-capture', '--execution-flow-dag', '--graph-output', graph,
                   '--output', output]
        self.logged(command, label + '-flow-analyzer')
        report = json.loads(output.read_text())
        native = json.loads(graph.read_text())
        query = next(a for a in native if a['producer'] == 'rustc-query-admission')
        for key in ('query_cpu_ownership', 'dispatch_result_provenance', 'cache_call_provenance'):
            self.assertNotIn(key, query, 'flow-only analysis acquired an unrelated opt-in')
        pool = next(a for a in native if a['producer'] == 'rustc-thread-pool-join')
        expected = next(a for a in evidence['public']
                        if a['producer'] == 'rustc-thread-pool-join')['execution_flow_dag']
        self.assertEqual(pool['execution_flow_dag'], expected)
        summary, = report['captures']
        compact = summary['execution_flow_dag']
        for key in ('scope', 'records_complete', 'known_cpu_work_ns', 'known_query_cpu_ns',
                    'longest_known_chain_cpu_ns'):
            self.assertEqual(compact[key], expected[key], key)
        named = [step for step in expected['longest_known_chain_steps']
                 if step['query_name'] is not None]
        named.sort(key=lambda step: (-step['cpu_ns'], step['node_id']))
        kinds = collections.Counter()
        for step in named:
            kinds[step['query_name']] += step['cpu_ns']
        ranked_kinds = [dict(query_name=name, cpu_ns=cost) for name, cost in
                        sorted(kinds.items(), key=lambda row: (-row[1], row[0]))]
        self.assertGreater(len(named), 0, 'actual causal chain needs named query CPU')
        self.assertEqual(compact['top_named_chain_cpu_segments'], named[:20])
        self.assertEqual(compact['named_chain_cpu_segments'], len(named))
        self.assertEqual(compact['omitted_named_chain_cpu_segments'], max(0, len(named) - 20))
        self.assertEqual(compact['named_chain_cpu_ns'], sum(kinds.values()))
        self.assertEqual(compact['top_chain_query_kinds'], ranked_kinds[:20])
        self.assertEqual(compact['chain_query_kinds'], len(ranked_kinds))
        self.assertEqual(compact['omitted_chain_query_kinds'], max(0, len(ranked_kinds) - 20))
        self.assertEqual(graph.stat().st_mode & 0o777, 0o600)
        self.assertEqual(output.stat().st_mode & 0o777, 0o600)
        self.assertEqual(report['graph_output']['sha256'], digest(graph))
        self.assertEqual(report['graph_output']['bytes'], graph.stat().st_size)
        before = digest(graph)
        collision = self.logged(command[:-1] + [self.root / (label + '-collision.json')],
                                label + '-flow-collision', expected=None)
        self.assertNotEqual(collision.returncode, 0)
        self.assertEqual(digest(graph), before)
        self.assertEqual(original, {p.name: digest(p) for p in sink.glob('*.json')})

    @unittest.skipUnless(os.environ.get('JR_FRONTEND_EXECUTION_FLOW'),
                         'requires coherent explicit execution flow compiler and consumer')
    def test_real_itoa_execution_flow(self):
        exhausted = Path(os.environ['JR_FRONTEND_FLOW_EXHAUSTION_REPLAY']).resolve(strict=True)
        exhausted_hash = digest(exhausted)
        result = self.logged([self.client, 'graph', exhausted, '--json', '--large-capture',
                              '--execution-flow-dag'], 'flow-native-exhaustion-graph')
        pool, = json.loads(result.stdout)
        flow = pool['execution_flow_dag']
        self.assertFalse(flow['records_complete'])
        for key in ('known_cpu_work_ns', 'known_query_cpu_ns', 'known_task_cpu_ns',
                    'known_opaque_cpu_ns', 'longest_known_chain_cpu_ns'):
            self.assertIsNone(flow[key], key)
        self.assertEqual(flow['longest_known_chain_path'], [])
        self.assertEqual(flow['longest_known_chain_steps'], [])
        self.assertEqual(digest(exhausted), exhausted_hash)
        output = self.root / 'libitoa-flow.rlib'
        observations = []
        cold = [self.compile(f'flow-cold-{n}', 4, output, capture=on, execution_flow=True)
                for n, on in enumerate((False, True, False))]
        self.assert_dispatch_control_neutrality([f'flow-cold-{n}' for n in range(3)])
        for first, other in zip(cold, cold[1:]):
            self.assertEqual(first[0], other[0], 'flow changed cold archive bytes')
            self.assertEqual(first[2:], other[2:], 'flow changed external compiler behavior')
        observations.append(cold[1][1])
        incremental = self.root / 'incremental-flow'
        self.compile('flow-warmup', 4, output, incremental=incremental, execution_flow=True)
        warm = [self.compile(f'flow-warm-{n}', 4, output, incremental=incremental,
                             capture=on, execution_flow=True)
                for n, on in enumerate((False, True, False))]
        self.assert_dispatch_control_neutrality([f'flow-warm-{n}' for n in range(3)])
        for first, other in zip(warm, warm[1:]):
            self.assertEqual(first[2:], other[2:], 'flow changed warm payload/diagnostics/run')
        observations.append(warm[1][1])
        observations.append(self.compile('flow-with-dispatch', 4, output, capture=True,
                                         execution_flow=True, dispatch=True)[1])
        for evidence in observations:
            self.assert_execution_flow_analyzer(evidence)
        self.assert_execution_flow_clock_offset(observations[1])
        self.assert_execution_flow_loss(cold[1][2])
        self.assertEqual(self.source_hashes, {
            str(p.relative_to(self.source_root)): digest(p)
            for p in sorted((self.source_root / 'src').rglob('*.rs'))})
        (self.root / 'execution-flow-acceptance.json').write_text(json.dumps({
            'observations': observations, 'cold_archive_byte_identity': True,
            'warm_rmeta_object_consumer_identity': True, 'full_pipeline_complete': False,
            'cpu_work_ns': None, 'execution_span_ns': None}, indent=2))

    def assert_execution_flow_clock_offset(self, evidence):
        """Translate query-relative time, never pool-local gates or binding points."""
        original = self.root / (evidence['label'] + '-capture')
        hashes = {p.name: digest(p) for p in original.glob('*.json')}
        copied = self.root / 'flow-nonzero-offset-copy'
        copied.mkdir()
        offset = 123456789
        for path in original.glob('*.json'):
            value = json.loads(path.read_text())
            if value['producer'] == 'rustc-query-admission':
                value['execution_clock']['pool_epoch_in_query_ns'] = offset
                for event in value['events'] + value['wait_events']:
                    event['at_ns'] += offset
            else:
                value['timing']['clock_bridge']['pool_epoch_in_query_ns'] = offset
            (copied / path.name).write_text(json.dumps(value))
        result = self.logged([self.client, 'graph', copied, '--json', '--large-capture',
                              '--execution-flow-dag'], 'flow-nonzero-offset-graph')
        public = json.loads(result.stdout)
        query = next(a for a in public if a['producer'] == 'rustc-query-admission')
        self.assertTrue(query['clock_alignment']['aligned'])
        flow = next(a for a in public if a['producer'] == 'rustc-thread-pool-join')['execution_flow_dag']
        expected = next(a for a in evidence['public']
                        if a['producer'] == 'rustc-thread-pool-join')['execution_flow_dag']
        self.assertEqual(flow, expected, 'clock-coordinate translation changed causal CPU evidence')
        self.assertEqual(hashes, {p.name: digest(p) for p in original.glob('*.json')})

    def assert_execution_flow_loss(self, compiler_args):
        sink = self.root / 'flow-bounded-loss-capture'
        control = str(sink) + '\n7\ntiming\noff\nquery_cpu\nexecution_flow\n'
        self.logged([self.callback, *compiler_args], 'flow-bounded-loss', control=control)
        replay = self.logged([self.client, 'graph', sink, '--json', '--large-capture',
                              '--execution-flow-dag'], 'flow-bounded-loss-graph')
        assessments = json.loads(replay.stdout)
        pool = next(a for a in assessments if a['producer'] == 'rustc-thread-pool-join')
        flow = pool['execution_flow_dag']
        self.assertFalse(flow['records_complete'])
        self.assertFalse(flow['flow_complete'])
        self.assertFalse(flow['full_pipeline_complete'])
        self.assertTrue(flow['lost_nodes'] or flow['lost_scopes'] or flow['lost_edges'])
        self.assertIsNone(flow['cpu_work_ns'])
        self.assertIsNone(flow['execution_span_ns'])
        self.assertIsNone(flow['parallelism'])
        raw_pool, = [json.loads(p.read_text()) for p in sink.glob('*.json')
                     if json.loads(p.read_text()).get('producer') == 'rustc-thread-pool-join']
        raw = raw_pool['execution_flow']
        nodes = {node['id']: node for node in raw['nodes']}
        segments = {row['segment_id']: row for row in raw_pool['timing']['segments']}
        self.assertTrue(any(scope['outcome'] in ('returned', 'unwound')
                            and scope['exit'] is not None and scope['exit'] not in nodes
                            and scope['exit'] < raw['node_allocator_high_water']
                            for scope in raw['scopes']),
                        'bounded capture must exercise a genuine lost terminal node')
        path = flow['longest_known_chain_path']
        steps = flow['longest_known_chain_steps']
        self.assertEqual(path, [step['node_id'] for step in steps])
        edges = {(edge['from'], edge['to']) for edge in raw['edges']}
        self.assertTrue(all(pair in edges for pair in zip(path, path[1:])))
        for step in steps:
            node = nodes[step['node_id']]
            self.assertNotEqual(node['kind'], 'cut', 'loss cannot bridge an unknown cut')
            if node['kind'] == 'cpu_segment':
                segment = segments[node['segment_id']]
                self.assertEqual(segment['status'], 'measured')
                self.assertEqual(step['cpu_ns'], segment['cpu_ns'])
            else:
                self.assertEqual(step['cpu_ns'], 0)
        if flow['longest_known_chain_cpu_ns'] is None:
            self.assertEqual(steps, [])
        else:
            self.assertEqual(sum(step['cpu_ns'] for step in steps),
                             flow['longest_known_chain_cpu_ns'])

    @unittest.skipUnless(os.environ.get('JR_FRONTEND_QUERY_CPU'),
                         'requires coherent explicit query CPU compiler and consumer')
    def test_real_itoa_query_cpu_ownership(self):
        self.replay_dispatch_legacy()
        output = self.root / 'libitoa-query-cpu.rlib'
        observations = []
        cold = [self.compile(f'query-cpu-cold-{n}', 4, output, capture=on, query_cpu=True)
                for n, on in enumerate((False, True, False))]
        self.assert_dispatch_control_neutrality([f'query-cpu-cold-{n}' for n in range(3)])
        for first, other in zip(cold, cold[1:]):
            self.assertEqual(first[0], other[0], 'query CPU changed cold archive bytes')
            self.assertEqual(first[2:], other[2:], 'query CPU changed external compiler behavior')
        observations.append(cold[1][1])
        incremental = self.root / 'incremental-query-cpu'
        self.compile('query-cpu-warmup', 4, output, incremental=incremental, query_cpu=True)
        warm = [self.compile(f'query-cpu-warm-{n}', 4, output, incremental=incremental,
                             capture=on, query_cpu=True)
                for n, on in enumerate((False, True, False))]
        self.assert_dispatch_control_neutrality([f'query-cpu-warm-{n}' for n in range(3)])
        off = self.compile('query-cpu-warm-off-control', 4, output, incremental=incremental,
                           query_cpu=True)
        for first, other in zip(warm, warm[1:]):
            self.assertEqual(first[2:], other[2:], 'query CPU changed warm payload/diagnostics/run')
        self.assertEqual(warm[-1][4:], off[4:])
        observations.append(warm[1][1])
        mixed = self.compile('query-cpu-with-dispatch', 4, output, capture=True,
                             query_cpu=True, dispatch=True)
        observations.append(mixed[1])
        for evidence in observations:
            self.assert_query_cpu_analyzer(evidence)
        self.assertEqual(self.source_hashes, {
            str(p.relative_to(self.source_root)): digest(p)
            for p in sorted((self.source_root / 'src').rglob('*.rs'))})
        (self.root / 'query-cpu-acceptance.json').write_text(json.dumps({
            'observations': observations, 'cold_archive_byte_identity': True,
            'warm_rmeta_object_consumer_identity': True, 'full_pipeline_complete': False,
            'cpu_work_ns': None, 'execution_span_ns': None}, indent=2))

    def assert_query_cpu_analyzer(self, evidence):
        """Exercise actual public analyzer, full artifact and named-kind rankings."""
        label = evidence['label']
        sink = self.root / (label + '-capture')
        original = {p.name: digest(p) for p in sink.glob('*.json')}
        output = self.root / (label + '-frontend-summary.json')
        graph = self.root / (label + '-frontend-graph.json')
        command = ['python3', self.analyzer, '--capture', sink, '--justrust', self.client,
                   '--large-capture', '--query-cpu-ownership', '--graph-output', graph,
                   '--output', output]
        self.logged(command, label + '-analyzer')
        report = json.loads(output.read_text())
        native = json.loads(graph.read_text())
        self.assertEqual(graph.stat().st_mode & 0o777, 0o600)
        self.assertEqual(output.stat().st_mode & 0o777, 0o600)
        self.assertEqual(report['graph_output']['sha256'], digest(graph))
        self.assertEqual(report['graph_output']['bytes'], graph.stat().st_size)
        actual = next(a for a in native if a['producer'] == 'rustc-query-admission')
        expected = next(a for a in evidence['public']
                        if a['producer'] == 'rustc-query-admission')
        self.assertNotIn('dispatch_result_provenance', actual,
                         'CPU-only root6 analysis acquired independent dispatch opt-in')
        self.assertNotIn('cache_call_provenance', actual)
        self.assertEqual(actual['query_cpu_ownership'], expected['query_cpu_ownership'])
        summary, = report['captures']
        costs = expected['query_cpu_ownership']
        compact = summary['query_cpu_ownership']
        self.assertTrue(compact['records_complete'], compact['issues'])
        self.assertEqual(compact['query_execution_cpu_ns'], costs['query_execution_cpu_ns'])
        self.assertEqual(compact['measured_segments'], costs['measured_segments'])
        self.assertEqual(compact['observed_query_owners'], len(costs['queries']))
        self.assertEqual(compact['top_queries'], costs['queries'][:20])
        self.assertEqual(compact['omitted_query_owners'], max(0, len(costs['queries']) - 20))
        kinds = collections.Counter()
        for row in costs['queries']:
            kinds[row['query_name']] += row['cpu_ns']
        ranked = [dict(query_name=name, cpu_ns=cost)
                  for name, cost in sorted(kinds.items(), key=lambda row: (-row[1], row[0]))]
        self.assertEqual(compact['observed_query_kinds'], len(kinds))
        self.assertEqual(compact['top_query_kinds'], ranked[:20])
        self.assertEqual(compact['omitted_query_kinds'], max(0, len(ranked) - 20))
        self.assertEqual(sum(kinds.values()), costs['query_execution_cpu_ns'])
        for scope in (report, summary):
            self.assertFalse(scope['full_pipeline_complete'])
            self.assertIsNone(scope['cpu_work_ns'])
            self.assertIsNone(scope['execution_span_ns'])
            self.assertIsNone(scope['theoretical_parallelism'])
        # An artifact collision must preserve both original files and avoid even
        # creating the fresh report. This executes the real CLI, not a mock.
        before = digest(graph), digest(output)
        collision = self.root / (label + '-must-not-exist.json')
        rejected = self.logged(command[:-1] + [collision], label + '-analyzer-collision', expected=None)
        self.assertNotEqual(rejected.returncode, 0)
        self.assertFalse(collision.exists())
        self.assertEqual(before, (digest(graph), digest(output)))
        self.assertEqual(original, {p.name: digest(p) for p in sink.glob('*.json')})
        self.assertEqual(digest(self.analyzer), self.analyzer_sha256)

    def assert_dispatch_receipts(self, query, assessments, sink, label, *, query_cpu=False):
        """Reconstruct every certified row directly from original native events."""
        reads = query['query_reads']
        self.assertEqual(reads['schema'], 2)
        self.assertTrue(reads['records_complete'])
        self.assertEqual(reads['lost_events'], 0)
        self.assertFalse(reads['semantic_complete'])
        events = reads['events']
        starts, terminals, publications = {}, {}, {}
        hits = set()
        facts = 0
        admitted = {(e['context_id'], e['job_id']) for e in query['events']
                    if e['kind'] == 'admitted'}
        for position, event in enumerate(events):
            self.assertIn('resolution', event)
            kind = event['kind']
            if kind == 'request_started':
                self.assertNotIn(event['request_id'], starts)
                starts[event['request_id']] = (position, event)
            elif kind in ('request_finished', 'request_interrupted'):
                self.assertNotIn(event['request_id'], terminals)
                terminals[event['request_id']] = (position, event)
            elif kind == 'cache_hit':
                hits.add(event['request_id'])
            elif kind == 'result_produced':
                key = (event['context_id'], event['dep_node_index'])
                self.assertNotIn(key, publications)
                publications[key] = (position, event)
            resolution = event['resolution']
            if resolution is not None:
                self.assertEqual(kind, 'request_finished')
                self.assertEqual(set(resolution), {'source', 'dep_node_index'})
                self.assertEqual(resolution['source'], 'resolved_dispatch')
                self.assertIn((event['mode'], event['outcome']),
                              (('get', 'value'), ('ensure_ok', 'ensured'),
                               ('ensure_result', 'ensured')))
                facts += resolution['dep_node_index'] is not None
        expected = []
        for request_id, (start_position, start) in sorted(starts.items()):
            terminal_position, terminal = terminals[request_id]
            self.assertLess(start_position, terminal_position)
            self.assertEqual((terminal['context_id'], terminal['consumer_job_id'], terminal['mode']),
                             (start['context_id'], start['consumer_job_id'], start['mode']))
            resolution = terminal['resolution']
            if request_id in hits:
                self.assertIsNone(resolution, 'first-cache path cannot also claim dispatch')
            if resolution is None:
                continue
            self.assertNotIn(request_id, hits)
            index = resolution['dep_node_index']
            context, consumer = start['context_id'], start['consumer_job_id']
            if index is None or context is None or consumer is None:
                continue
            publication = publications.get((context, index))
            if publication is None:
                continue
            publication_position, produced = publication
            producer = produced['producer_job_id']
            if publication_position >= terminal_position:
                continue
            if (context, consumer) not in admitted or (context, producer) not in admitted:
                continue
            expected.append({
                'request_id': request_id, 'context_id': context, 'consumer_job_id': consumer,
                'result_id': produced['result_id'], 'producer_job_id': producer,
                'dep_node_index': index, 'request_start_event_index': start_position,
                'publication_event_index': publication_position,
                'request_terminal_event_index': terminal_position})
        public = next(a for a in assessments if a['producer'] == 'rustc-query-admission')
        report = public['dispatch_result_provenance']
        self.assertEqual(report['scope'], 'observed-dispatch-result-availability-not-execution-dag')
        self.assertEqual(report['relations'], expected)
        self.assertEqual(report['observed_requests'], len(starts))
        self.assertEqual(report['observed_dispatch_resolutions'],
                         sum(e['resolution'] is not None for e in events))
        self.assertEqual(report['resolved_requests'], len(expected))
        self.assertEqual(report['unresolved_requests'], len(starts) - len(expected))
        self.assertEqual(sum(report['unresolved_reasons'].values()), report['unresolved_requests'])
        self.assertEqual(report['distinct_publishing_job_relations'], len({
            (r['context_id'], r['producer_job_id'], r['consumer_job_id']) for r in expected}))
        self.assertTrue(report['records_complete'])
        self.assertNotIn('cache_call_provenance', public)
        # Both analyses are independently opted in and cannot alter one another.
        command = [self.client, 'graph', sink, '--json', '--large-capture',
                   '--dispatch-result-provenance', '--cache-call-provenance']
        if query_cpu:
            command.append('--query-cpu-ownership')
        both = json.loads(self.logged(command, label + '-both-graph').stdout)
        both_query = next(a for a in both if a['producer'] == 'rustc-query-admission')
        cache = both_query.pop('cache_call_provenance')
        self.assertEqual(both, assessments)
        self.assertGreater(cache['observed_cache_hits'], 0)
        for event in events:
            if event['mode'] == 'feed' or event['kind'] != 'request_finished':
                self.assertIsNone(event['resolution'])
        return {'nonnull_dispatch_indices': facts, 'resolved_requests': len(expected),
                'observed_dispatch_resolutions': report['observed_dispatch_resolutions'],
                'first_cache_requests': len(hits), 'all_rows_independently_verified': True}

    def replay_dispatch_legacy(self):
        for schema in (4, 5):
            supplied = os.environ.get(f'JR_FRONTEND_QUERY{schema}_REPLAY')
            self.assertTrue(supplied, f'dispatch acceptance requires frozen query{schema} replay')
            reference = Path(supplied).resolve(strict=True)
            before = {p.name: digest(p) for p in reference.glob('*.json')}
            self.assertTrue(before)
            label = reference.name.removesuffix('-capture')
            expected = json.loads((reference.parent / (label + '-assessment.json')).read_text())
            command = [self.client, 'graph', reference, '--json', '--large-capture']
            observed = json.loads(self.logged(command, f'dispatch-query{schema}-replay').stdout)
            self.assertEqual(observed, expected, 'legacy default assessment changed')
            self.assertEqual(before, {p.name: digest(p) for p in reference.glob('*.json')})
            for row in observed:
                self.assertNotIn('dispatch_result_provenance', row)
                self.assertNotIn('cache_call_provenance', row)
            (self.root / f'dispatch-query{schema}-replay-hashes.json').write_text(
                json.dumps(before, indent=2))

    def assert_dispatch_control_neutrality(self, labels):
        logs = [json.loads((self.root / (label + '.json')).read_text()) for label in labels]
        for first, other in zip(logs, logs[1:]):
            self.assertEqual(first['command'], other['command'], 'capture changed process argv')
            self.assertEqual(first['environment_sha256'], other['environment_sha256'],
                             'capture changed process environment')
            self.assertEqual(first['stdout'], other['stdout'])
            self.assertEqual(first['stderr'], other['stderr'])
        self.assertEqual(logs[0]['control_sha256'], logs[-1]['control_sha256'])
        self.assertNotEqual(logs[0]['control_sha256'], logs[1]['control_sha256'])

    @unittest.skipUnless(os.environ.get('JR_FRONTEND_DISPATCH_RESOLUTIONS'),
                         'requires coherent explicit dispatch compiler and consumer')
    def test_real_itoa_dispatch_resolutions(self):
        self.replay_dispatch_legacy()
        observations = []
        for threads in (1, 4):
            output = self.root / f'libitoa-dispatch-t{threads}.rlib'
            cold = [self.compile(f'dispatch-cold-t{threads}-{n}', threads, output,
                                 capture=on, dispatch=True)
                    for n, on in enumerate((False, True, False))]
            self.assert_dispatch_control_neutrality([f'dispatch-cold-t{threads}-{n}' for n in range(3)])
            for first, other in zip(cold, cold[1:]):
                self.assertEqual(first[0], other[0], 'dispatch changed cold archive bytes')
                self.assertEqual(first[2:], other[2:], 'dispatch changed argv/diagnostics/payload/run')
            evidence = cold[1][1]
            self.assertGreater(evidence['dispatch']['nonnull_dispatch_indices'], 0)
            self.assertGreater(evidence['dispatch']['resolved_requests'], 0)
            query_public = next(a for a in evidence['public']
                                if a['producer'] == 'rustc-query-admission')
            self.assertGreater(query_public['query_reads']['unbound_read_reasons']['disabled'], 0)
            self.assertFalse(query_public['query_reads']['semantic_complete'])
            observations.append(evidence)
            incremental = self.root / f'dispatch-incremental-t{threads}'
            self.compile(f'dispatch-warmup-t{threads}', threads, output,
                         incremental=incremental, dispatch=True)
            warm = [self.compile(f'dispatch-warm-t{threads}-{n}', threads, output,
                                 incremental=incremental, capture=on, dispatch=True)
                    for n, on in enumerate((False, True, False))]
            self.assert_dispatch_control_neutrality([f'dispatch-warm-t{threads}-{n}' for n in range(3)])
            baseline = self.compile(f'dispatch-warm-off-control-t{threads}', threads, output,
                                    incremental=incremental, dispatch=True)
            for first, other in zip(warm, warm[1:]):
                self.assertEqual(first[2:], other[2:], 'warm dispatch changed observable payload')
            self.assertEqual(warm[-1][4:], baseline[4:], 'OFF/OFF incremental baseline unstable')
            self.assertEqual(warm[1][1]['dispatch']['observed_dispatch_resolutions'], 0,
                             'unsupported incremental execution acquired dispatch metadata')
            observations.append(warm[1][1])
        mixed = self.compile('dispatch-with-timing', 4, self.root / 'libitoa-dispatch-mixed.rlib',
                             capture=True, timing=True, dispatch=True)
        self.assertGreater(mixed[1]['dispatch']['resolved_requests'], 0)
        observations.append(mixed[1])
        self.assertEqual(self.source_hashes, {
            str(p.relative_to(self.source_root)): digest(p)
            for p in sorted((self.source_root / 'src').rglob('*.rs'))})
        (self.root / 'dispatch-acceptance.json').write_text(json.dumps({
            'observations': observations, 'cold_archive_byte_identity': True,
            'warm_rmeta_object_consumer_identity': True,
            'full_pipeline_complete': False, 'cpu_work_ns': None,
            'execution_span_ns': None}, indent=2))

    def replay_query4(self):
        """Replay original accepted bytes, never reserialize or amend the old capture."""
        supplied = os.environ.get('JR_FRONTEND_QUERY4_REPLAY')
        self.assertTrue(supplied, 'timed acceptance requires explicit frozen JR_FRONTEND_QUERY4_REPLAY')
        reference = Path(supplied).resolve(strict=True)
        before = {p.name: digest(p) for p in reference.glob('*.json')}
        self.assertTrue(before)
        label = reference.name.removesuffix('-capture')
        expected_path = reference.parent / (label + '-assessment.json')
        self.assertTrue(expected_path.is_file(), 'original accepted public assessment required')
        expected = json.loads(expected_path.read_text())
        result = self.logged([self.client, 'graph', reference, '--json'], 'timed-query4-replay')
        observed = json.loads(result.stdout)
        # New optional assessments may be null, never falsely measured/aligned.
        for rows in (expected, observed):
            for row in rows:
                for field in ('clock_alignment', 'observed_cpu'):
                    self.assertIsNone(row.get(field), 'legacy capture acquired timing authority')
                    row.pop(field, None)
        self.assertEqual(observed, expected, 'legacy non-timing assessment changed')
        self.assertEqual(before, {p.name: digest(p) for p in reference.glob('*.json')})
        (self.root / 'timed-query4-replay-hashes.json').write_text(json.dumps(before, indent=2))

    @unittest.skipUnless(os.environ.get('JR_FRONTEND_TIMING'),
                         'requires coherent opt-in timing compiler and consumer')
    def test_real_itoa_timing_cold_and_incremental(self):
        self.replay_query4()
        output = self.root / 'libitoa-timed.rlib'
        observations = []
        controls = [self.compile(f'timed-cold-t4-{n}', 4, output, capture=on, large=True, timing=True)
                    for n, on in enumerate((False, True, False))]
        for first, other in zip(controls, controls[1:]):
            self.assertEqual(first[0], other[0], 'timed cold rlib bytes changed')
            self.assertEqual(first[2:], other[2:], 'timed cold argv/diagnostics/object/consumer changed')
        observations.append(controls[1][1])
        incremental = self.root / 'incremental-timed'
        self.compile('timed-warmup', 4, output, incremental=incremental, timing=True)
        controls = [self.compile(f'timed-warm-t4-{n}', 4, output, incremental=incremental,
                                 capture=on, timing=True)
                    for n, on in enumerate((False, True, False))]
        off_control = self.compile('timed-warm-off-control', 4, output,
                                   incremental=incremental, timing=True)
        for first, other in zip(controls, controls[1:]):
            self.assertEqual(first[2:], other[2:], 'timed warm argv/diagnostics/object/consumer changed')
        self.assertEqual(controls[-1][4:], off_control[4:])
        observations.append(controls[1][1])
        observations[-1]['whole_archive_byte_identity'] = len({c[0] for c in controls}) == 1
        observations[-1]['off_off_whole_archive_byte_identity'] = controls[-1][0] == off_control[0]
        self.assertEqual(self.source_hashes, {
            str(p.relative_to(self.source_root)): digest(p)
            for p in sorted((self.source_root / 'src').rglob('*.rs'))}, 'real sources modified')
        (self.root / 'timing-acceptance.json').write_text(json.dumps({
            'observations': observations, 'cold_archive_byte_identity': True,
            'warm_rmeta_object_consumer_identity': True, 'cpu_work_ns': None,
            'execution_span_ns': None, 'full_pipeline_complete': False}, indent=2))

    @unittest.skipUnless(os.environ.get('JR_FRONTEND_LARGE_CAPTURE'),
                         'requires rebuilt capacity API and installed large consumer')
    def test_real_itoa_large_cold_mapping(self):
        output = self.root / 'libitoa-large.rlib'
        controls = [self.compile(f'large-cold-t4-{n}', 4, output, capture=on, large=True)
                    for n, on in enumerate((False, True, False))]
        for first, other in zip(controls, controls[1:]):
            self.assertEqual(first[0], other[0], 'large cold rlib bytes changed')
            self.assertEqual(first[2:], other[2:], 'large capture compiler/consumer neutrality')
        summary = controls[1][1]
        self.assertEqual(summary['read_lost_events'], 0, 'real source read budget insufficient')
        self.assertEqual(summary['binding_lost_events'], 0, 'real source point budget insufficient')
        self.assertTrue(summary['binding_records_complete'])
        self.assertGreater(summary['read_records'], 100000, 'real oversized capture not exercised')
        self.assertGreater(summary['pool_root_events'], 0, 'actual native frontend root required')
        self.assertEqual(summary['pool_dropped_events'], 0, 'native task inventory loss')
        self.assertEqual(self.source_hashes, {
            str(p.relative_to(self.source_root)): digest(p)
            for p in sorted((self.source_root / 'src').rglob('*.rs'))}, 'real sources modified')
        (self.root / 'large-acceptance.json').write_text(json.dumps(summary, indent=2))

    def test_real_itoa_cold_and_incremental_mapping(self):
        observations = []
        for threads in (1, 4):
            output = self.root / ('libitoa-t' + str(threads) + '.rlib')
            # Cold runs have no incremental storage, fixed source/output/arguments.
            controls = [self.compile(f'cold-t{threads}-{n}', threads, output, capture=on)
                        for n, on in enumerate((False, True, False))]
            for first, other in zip(controls, controls[1:]):
                self.assertEqual(first[0], other[0], 'cold capture changed actual rlib bytes')
                self.assertEqual(first[5], other[5], 'cold capture changed executed consumer bytes')
                self.assertEqual(first[2:4], other[2:4], 'compiler arguments/diagnostics changed')
            observations.append(controls[1][1])
            incremental = self.root / ('incremental-t' + str(threads))
            self.compile(f'warmup-t{threads}', threads, output, incremental=incremental)
            controls = [self.compile(f'incremental-t{threads}-{n}', threads, output,
                                    incremental=incremental, capture=on)
                        for n, on in enumerate((False, True, False))]
            # Extra OFF/OFF control tests baseline archive instability independently.
            off_control = self.compile(f'incremental-t{threads}-off-control', threads, output,
                                       incremental=incremental)
            for first, other in zip(controls, controls[1:]):
                self.assertEqual(first[4], other[4], 'warm capture changed rmeta/object bytes')
                self.assertEqual(first[5], other[5], 'warm capture changed executed consumer bytes')
                self.assertEqual(first[2:4], other[2:4], 'compiler arguments/diagnostics changed')
            self.assertEqual(controls[-1][4:], off_control[4:], 'OFF/OFF payload/executable baseline unstable')
            observations.append(controls[1][1])
            observations[-1]['whole_archive_byte_identity'] = len({c[0] for c in controls}) == 1
            observations[-1]['off_off_whole_archive_byte_identity'] = controls[-1][0] == off_control[0]
            observations[-1]['rmeta_object_payload_identity'] = True
            observations[-1]['consumer_executable_identity'] = True
            observations[-1]['excluded_payload_identity'] = 'lib.rmeta-link: raw hashes retained'
        self.assertEqual(self.source_hashes, {
            str(p.relative_to(self.source_root)): digest(p)
            for p in sorted((self.source_root / 'src').rglob('*.rs'))}, 'real sources modified')
        (self.root / 'acceptance.json').write_text(json.dumps({
            'observations': observations, 'cold_archive_byte_identity': True,
            'incremental_rmeta_object_payload_identity': True,
            'consumer_executable_identity': True,
            'incremental_archive_and_link_metadata_byte_identity': all(
                row['whole_archive_byte_identity'] for row in observations
                if row['label'].startswith('incremental')),
            'full_pipeline_complete': False, 'cpu_work_ns': None, 'execution_span_ns': None
        }, indent=2))


class FixtureControlSafety(unittest.TestCase):
    def test_flow_positive_acceptance_refuses_all_unknown(self):
        from bench.test_execution_flow_oracle import fixture
        from bench.execution_flow_oracle import execution_flow_oracle
        query, pool = fixture()
        flow = execution_flow_oracle(query, pool)
        flow['issues'] = []
        reports = [dict(producer='rustc-thread-pool-join', execution_flow_dag=flow)]
        harness = FrontendMappingNative('test_real_itoa_execution_flow')
        harness.assert_execution_flow_receipts(query, [pool], reports)
        flow.update(records_complete=False, known_cpu_work_ns=None,
                    known_query_cpu_ns=None, longest_known_chain_cpu_ns=None,
                    longest_known_chain_path=[], longest_known_chain_steps=[])
        with self.assertRaises(AssertionError):
            harness.assert_execution_flow_receipts(query, [pool], reports)

    def test_query_cpu_positive_acceptance_refuses_all_unknown(self):
        harness = FrontendMappingNative('test_real_itoa_query_cpu_ownership')
        query = {'events': []}
        pools = [{'timing': {'segments': []}}]
        assessments = [
            dict(producer='rustc-query-admission', query_cpu_ownership=dict(
                scope='observed-query-execution-cpu', measured_segments=0,
                query_execution_cpu_ns=None, queries=[], records_complete=False,
                issues=['unavailable'])),
            dict(producer='rustc-thread-pool-join', observed_cpu=dict(
                query_execution_cpu_ns=None, issues=['unavailable']))]
        with self.assertRaises(AssertionError):
            harness.assert_query_cpu_receipts(query, pools, assessments)

    def test_legacy_and_dispatch_control_packets_are_separate(self):
        self.assertEqual(fixture_controls(None), '-\ndefault\n')
        self.assertEqual(fixture_controls('/capture'), '/capture\ndefault\n')
        self.assertEqual(fixture_controls('/capture', large=True), '/capture\n1000000\n')
        self.assertEqual(fixture_controls('/capture', timing=True), '/capture\n1000000\ntiming\n')
        self.assertEqual(fixture_controls('/capture', dispatch=True), '/capture\n1000000\noff\ndispatch\n')
        self.assertEqual(fixture_controls('/capture', timing=True, dispatch=True),
                         '/capture\n1000000\ntiming\ndispatch\n')
        self.assertEqual(fixture_controls('/capture', query_cpu=True),
                         '/capture\n1000000\ntiming\noff\nquery_cpu\n')
        self.assertEqual(fixture_controls('/capture', dispatch=True, query_cpu=True),
                         '/capture\n1000000\ntiming\ndispatch\nquery_cpu\n')
        self.assertEqual(fixture_controls('/capture', execution_flow=True),
                         '/capture\n1000000\ntiming\noff\nquery_cpu\nexecution_flow\n')
        self.assertEqual(fixture_controls('/capture', dispatch=True, execution_flow=True),
                         '/capture\n1000000\ntiming\ndispatch\nquery_cpu\nexecution_flow\n')

    def test_foreign_descriptor_is_not_overwritten(self):
        read_fd, write_fd = os.pipe2(os.O_CLOEXEC)
        owned = None
        try:
            owned = fcntl.fcntl(read_fd, fcntl.F_DUPFD_CLOEXEC, 198)
            if owned != 198:
                self.skipTest('caller already owns fd198: do not overwrite')
            before = os.fstat(owned)
            flags = fcntl.fcntl(owned, fcntl.F_GETFD)
            harness = FrontendMappingNative('test_real_itoa_cold_and_incremental_mapping')
            with self.assertRaisesRegex(AssertionError, 'foreign fd198 occupied'):
                harness.logged(['must-not-execute'], 'collision', control='-\ndefault\n')
            self.assertEqual(os.fstat(owned), before)
            self.assertEqual(fcntl.fcntl(owned, fcntl.F_GETFD), flags)
            os.write(write_fd, b'foreign-descriptor-preserved')
            self.assertEqual(os.read(owned, 64), b'foreign-descriptor-preserved')
        finally:
            if owned is not None:
                os.close(owned)
            os.close(read_fd)
            os.close(write_fd)


if __name__ == '__main__':
    unittest.main()
