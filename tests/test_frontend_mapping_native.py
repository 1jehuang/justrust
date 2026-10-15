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
        shutil.copyfile(fixture, self.root / fixture.name)
        (self.root / 'provenance.json').write_text(json.dumps({
            'crate': 'itoa', 'version': versions[0], 'edition': self.edition,
            'source_root': str(self.source_root), 'source_hashes': self.source_hashes,
            'default_features': [], 'local_reason': 'accepted private compiler diagnostic',
            'artifacts': {str(p): digest(p) for p in
                          (self.rustc, self.stage0, driver, meta, fixture, repo / 'Cargo.lock',
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
        if os.environ.get('JR_FRONTEND_SCOPE_API'):
            build_args.append('--cfg=frontend_scope_api')
        self.logged(build_args, 'driver-build')

    def compile(self, label, threads, output, incremental=None, capture=False, large=False,
                timing=False):
        sink = self.root / (label + '-capture') if capture else None
        args = ['--sysroot', self.sysroot, '--crate-name=itoa', '--crate-type=rlib',
                '--edition=' + self.edition, '--emit=link', '-Copt-level=0', '-Cdebuginfo=0',
                '-Zthreads=' + str(threads), '--check-cfg=cfg(feature, values("no-panic"))',
                self.source, '-o', output]
        if incremental is not None:
            args.append('-Cincremental=' + str(incremental))
        control = str(sink or '-') + '\n' + ('1000000' if large or timing else 'default') + '\n'
        if timing:
            control += 'timing\n'
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
        if large:
            rejected = self.logged(graph_command, label + '-default-graph', expected=None)
            self.assertNotEqual(rejected.returncode, 0, 'oversized capture default guard must reject')
            graph_command.append('--large-capture')
        if timing and not large:
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
        if timing:
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
        if timing:
            summary['timing'] = self.assert_timed_receipts(query, pools, assessments, sink, label,
                                                        large=large)
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

    def assert_timed_point_inventory(self, query):
        """Same allocator contract as query4, without rewriting the actual query5 header."""
        self.assertEqual(query['schema'], 5)
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

    def assert_timed_receipts(self, query, pools, assessments, sink, label, *, large):
        self.assertEqual(len(pools), 1, 'one actual timed native pool required')
        pool = pools[0]
        self.assertEqual(pool['schema'], 3)
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
        self.assertEqual(timing['schema'], 1)
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
        task_cpu = opaque_cpu = measured = failed = 0
        envelope_uncertain = False
        for segment in segments:
            sid = segment['segment_id']
            self.assertNotIn(sid, ids)
            ids.add(sid)
            self.assertTrue(1 <= sid < timing['segment_allocator_high_water'])
            self.assertTrue(1 <= segment['thread_id'] < timing['thread_allocator_high_water'])
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
            self.assertGreater(task_cpu + opaque_cpu, 0, 'actual busy compiler must have scoped CPU')
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
            self.assertFalse(cpu['records_complete'])
            self.assertTrue(cpu['issues'], 'unknown quantities need an explicit cause')
        else:
            self.assertEqual(cpu['observed_cpu_ns'], task_cpu + opaque_cpu)
            self.assertEqual(cpu['task_cpu_ns'], task_cpu)
            self.assertEqual(cpu['opaque_cpu_ns'], opaque_cpu)
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
        rejected = self.logged([self.client, 'graph', negative, '--json', '--large-capture'],
                               label + '-unilateral-clock-graph', expected=None)
        if rejected.returncode == 0:
            changed = json.loads(rejected.stdout)
            changed_query = next(a for a in changed if a['producer'] == 'rustc-query-admission')
            self.assertFalse(changed_query['clock_alignment']['aligned'],
                             'unilateral clock declaration acquired reciprocal authority')
            changed_pool = next(a for a in changed if a['producer'] == 'rustc-thread-pool-join')
            self.assertEqual(changed_pool['observed_cpu'], cpu,
                             'foreign query clock erased independent pool CPU evidence')
            self.assertEqual(changed_query['task_bindings'], query_public['task_bindings'],
                             'clock mismatch altered independent point authority')
        return {'scoped_observed_cpu_ns': cpu['observed_cpu_ns'], 'task_cpu_ns': cpu['task_cpu_ns'],
                'opaque_cpu_ns': cpu['opaque_cpu_ns'], 'segments': len(segments),
                'aggregate_unknown': aggregates_unknown, 'cpu_issues': cpu['issues'],
                'measured_segments': measured, 'failed_segments': failed,
                'observed_threads': len(per_thread), 'clock_aligned': True,
                'unilateral_clock_rejected_or_unaligned': True}

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
