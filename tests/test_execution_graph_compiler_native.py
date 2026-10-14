#!/usr/bin/env python3
"""Opt-in real compiler binding acceptance, never a toolchain build helper.

Run only after the combined native compiler/std and Cargo builds finish:
  JUSTRUST_NATIVE_CARGO=/absolute/path/to/cargo \
  JUSTRUST_NATIVE_RUSTC=/absolute/path/to/stage1/bin/rustc \
  JUSTRUST_NATIVE_CLIENT=/absolute/path/to/justrust \
  python3 tests/test_execution_graph_compiler_native.py -v

Only missing Cargo/rustc pins skip. Bad binaries, capability or client fail.
JUSTRUST_NATIVE_CLIENT defaults to installed justrust. Optional
JUSTRUST_NATIVE_EVIDENCE_DIR retains command logs, fixtures and raw receipts.
Cargo runs directly, locally and unwrapped with explicit RUSTC. No justrust
build routing, forced cache disabling, bootstrap, or rustflags are injected.
Byte identity and fresh-unit observations are correctness checks, not speed claims.
"""
import hashlib
import copy
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time
import unittest

from test_execution_graph_task_binding_native import assert_binding_inventory


PROFILE = 'buildrunner-compiler-launch-v1'
CAPABILITY = 'jr-query-admission-binding: reserved-fd-v2,query-schema=4'


@unittest.skipUnless(os.environ.get('JUSTRUST_NATIVE_CARGO') and
                     os.environ.get('JUSTRUST_NATIVE_RUSTC'),
                     'requires JUSTRUST_NATIVE_CARGO and JUSTRUST_NATIVE_RUSTC')
class NativeCompilerBindingAcceptance(unittest.TestCase):
    def setUp(self):
        evidence = os.environ.get('JUSTRUST_NATIVE_EVIDENCE_DIR')
        if evidence:
            Path(evidence).mkdir(parents=True, exist_ok=True)
            self.root = Path(tempfile.mkdtemp(prefix='compiler-native-', dir=evidence))
        else:
            temporary = tempfile.TemporaryDirectory(
                prefix='compiler-native-', dir=os.environ.get('JCODE_SCRATCH_DIR'))
            self.addCleanup(temporary.cleanup)
            self.root = Path(temporary.name)
        self.root = self.root.resolve()
        self.serial = 0
        self.cargo = str(Path(os.environ['JUSTRUST_NATIVE_CARGO']).resolve(strict=True))
        self.rustc = str(Path(os.environ['JUSTRUST_NATIVE_RUSTC']).resolve(strict=True))
        self.client = os.environ.get('JUSTRUST_NATIVE_CLIENT') or shutil.which('justrust')
        self.assertTrue(self.client, 'installed/new justrust graph consumer required')
        for binary in (self.cargo, self.rustc):
            with open(binary, 'rb') as stream:
                self.assertEqual(stream.read(4), b'\x7fELF',
                                 'pin actual Linux native binaries, not shell wrappers')
            self.assertTrue(os.access(binary, os.X_OK), binary)
        self.env = os.environ.copy()
        for key in ('JUSTRUST_EXECUTION_GRAPH_DIR', 'RUSTC_WRAPPER',
                    'RUSTC_WORKSPACE_WRAPPER', 'RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS',
                    'CARGO_BUILD_RUSTFLAGS', 'CARGO_BUILD_RUSTC_WRAPPER',
                    'CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER', 'RUSTC_BOOTSTRAP',
                    'CARGO_INCREMENTAL'):
            self.env.pop(key, None)
        # A new Cargo home excludes inherited config wrappers and registry state.
        # Dependency-free --offline builds need neither downloads nor cache switches.
        self.env.update(RUSTC=self.rustc, CARGO_HOME=str(self.root / 'cargo-home'),
                        CARGO_TARGET_DIR=str(self.root / 'target'))
        self.write('Cargo.toml', '[package]\nname="compiler-native-fixture"\n'
                   'version="0.1.0"\nedition="2024"\n[workspace]\n')
        self.write('.cargo/config.toml', '[build]\nrustc-wrapper=""\n'
                   'rustc-workspace-wrapper=""\n')
        self.original = ('const _: () = assert!(option_env!("JUSTRUST_EXECUTION_GRAPH_DIR").is_none());\n'
                         'fn main() { assert_eq!((0..10u32).sum::<u32>(), 45); }\n')
        self.source = self.root / 'src/main.rs'
        self.write('src/main.rs', self.original)
        self.binary = self.root / 'target/debug/compiler-native-fixture'
        version = self.run_command([self.rustc, '-vV'])
        markers = [line for line in version.stdout.splitlines()
                   if line.startswith('jr-query-admission-binding:')]
        self.assertEqual(markers, [CAPABILITY], version.stdout + version.stderr)
        print(json.dumps({'evidence_dir': str(self.root), 'retained': bool(evidence),
                          'cargo': self.cargo, 'rustc': self.rustc,
                          'client': self.client}), flush=True)

    def write(self, relative, text):
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)

    def run_command(self, command, capture=None, success=True, check=True):
        env = self.env.copy()
        if capture is not None:
            env['JUSTRUST_EXECUTION_GRAPH_DIR'] = str(capture)
        started = time.monotonic()
        result = subprocess.run([str(arg) for arg in command], cwd=self.root, env=env,
                                capture_output=True, text=True, timeout=240)
        self.serial += 1
        log = {'command': [str(arg) for arg in command], 'exit_code': result.returncode,
               'stdout': result.stdout, 'stderr': result.stderr,
               'wall_seconds': time.monotonic() - started,
               'capture': str(capture) if capture is not None else None,
               'rustc': env['RUSTC'], 'wrappers': {
                   key: env.get(key) for key in ('RUSTC_WRAPPER', 'RUSTC_WORKSPACE_WRAPPER')},
               'source_sha256': hashlib.sha256(self.source.read_bytes()).hexdigest()}
        self.write(f'commands/{self.serial:03}.json', json.dumps(log, indent=2))
        if check:
            output = result.stdout + result.stderr
            if success:
                self.assertEqual(result.returncode, 0, output)
            else:
                self.assertNotEqual(result.returncode, 0, output)
        return result

    def build(self, capture=None, success=True):
        return self.run_command([self.cargo, 'build', '--offline', '-vv'], capture, success)

    def private_sink(self, name):
        sink = self.root / name
        sink.mkdir(mode=0o700)
        sink.chmod(0o700)
        return sink

    def digest(self):
        return hashlib.sha256(self.binary.read_bytes()).hexdigest()

    def force_same_source_rebuild(self):
        # Change only the input mtime, not bytes, flags, profile or target directory.
        # A one-second gap also works on coarse-mtime filesystems.
        previous = self.source.stat().st_mtime_ns
        while True:
            time.sleep(0.05)
            self.source.touch()
            if self.source.stat().st_mtime_ns > previous + 1_000_000_000:
                break

    def manifests(self, sink):
        selected = [(path, json.loads(path.read_text()))
                    for path in sorted(sink.glob('compiler-owner-*.manifest.json'))]
        self.assertTrue(selected, 'native Cargo compiler-launch manifests required')
        for _, manifest in selected:
            self.assertEqual(manifest['profile'], PROFILE)
        return selected

    def assess(self, sink, manifest_path, complete=None, may_reject=False):
        result = self.run_command([self.client, 'graph', sink, '--compiler-manifest',
                                   manifest_path, '--json'], check=not may_reject)
        if result.returncode:
            self.assertTrue(may_reject, result.stdout + result.stderr)
            return None
        assessment = json.loads(result.stdout)
        self.assertEqual(assessment['selection'], 'explicit-compiler-authority')
        self.assertFalse(assessment['full_pipeline_complete'])
        self.assertIsNone(assessment['cpu_work_ns'])
        self.assertIsNone(assessment['execution_span_ns'])
        if complete is not None:
            self.assertEqual(assessment['structural_complete'], complete, result.stdout)
        return assessment

    def inspect_bound(self, sink, complete=True):
        manifests = self.manifests(sink)
        launches = []
        for path, manifest in manifests:
            coverage = self.assess(sink, path, complete=complete)
            self.assertEqual(coverage['expected_launches'], len(manifest['launches']))
            for launch in manifest['launches']:
                launches.append((path, manifest, launch))
                self.assertTrue(launch['spawned'])
                self.assertTrue(launch['terminal'])
                entry = json.loads((sink / (launch['entry_capture_id'] + '.json')).read_text())
                query = json.loads((sink / (launch['capture_id'] + '.json')).read_text())
                self.assertEqual(entry['producer'], 'rustc-query-admission-entry')
                self.assertEqual(query['producer'], 'rustc-query-admission')
                self.assertEqual(entry['query_receipt_schema'], 4)
                self.assertEqual(query['schema'], 4)
                assert_binding_inventory(self, query)
                self.assertEqual(entry['expected_query_captures'], 1)
                self.assertEqual(entry['query_capture_ids'], [launch['capture_id']])
                self.assertEqual(entry['capture_id'], launch['entry_capture_id'])
                self.assertEqual(query['capture_id'], launch['capture_id'])
                self.assertEqual(entry['run_id'], manifest['run_id'])
                self.assertEqual(entry['invocation_id'], launch['invocation_id'])
                self.assertEqual(entry['parent_invocation_id'], manifest['authority_root'])
                self.assertEqual(entry['parent_task_id'], launch['parent_task_id'])
                self.assertFalse(entry['full_pipeline_complete'])
                self.assertFalse(query['full_pipeline_complete'])
                self.assertIn('query_reads', query, 'raw schema4 read accounting required')
                if complete:
                    self.assertEqual(launch['outcome'], 'success')
                    self.assertEqual(launch['wait_status'], 0)
        self.assertTrue(launches, 'actual native launches required, not just fresh units')
        return launches

    def test_selected_native_pool_point_authority(self):
        # Keep real parallel bodies, but avoid assert_eq formatting/type-query
        # fanout that exhausts the independent read recorder on a cold build.
        self.source.write_text('\n'.join(f'fn f{i}() -> u64 {{ {i} }}' for i in range(8)) +
                               '\nfn main() { assert!(' +
                               '+'.join(f'f{i}()' for i in range(8)) + ' == 28); }\n')
        self.env['RUSTFLAGS'] = '-Zthreads=4'
        sink = self.private_sink('parallel-bound')
        self.build(sink)
        self.run_command([self.binary])
        self.inspect_point_authority_copies(sink, self.inspect_bound(sink))

    def inspect_point_authority_copies(self, sink, bound):
        manifest_path, _, launch = next((p, m, l) for p, m, l in bound
                                       if json.loads((sink / (l['capture_id'] + '.json')).read_text())['task_bindings']['pool'] is not None)
        query_path = sink / (launch['capture_id'] + '.json')
        query = json.loads(query_path.read_text())
        pool_id = query['task_bindings']['pool']['capture_id']
        pool_path = next(p for p in sink.glob('*.json')
                         if json.loads(p.read_text()).get('capture_id') == pool_id)
        pool = json.loads(pool_path.read_text())
        self.assertEqual(pool['schema'], 2)
        self.assertEqual(pool['query_capture_id'], query['capture_id'])
        baseline = self.assess(sink, manifest_path)
        binding = baseline['task_bindings'][query['capture_id']]
        self.assertTrue(binding['graph_binding_valid'], baseline)
        for label in ('missing-pool', 'duplicate-pool', 'foreign-capture', 'stale-observer',
                      'foreign-query', 'outside-interval', 'remapped-task'):
            altered = self.root / ('selected-' + label)
            shutil.copytree(sink, altered)
            q, p = copy.deepcopy(query), copy.deepcopy(pool)
            if label == 'missing-pool':
                (altered / pool_path.name).unlink()
            elif label == 'duplicate-pool':
                (altered / 'duplicate-pool.json').write_text(json.dumps(p))
            elif label == 'foreign-capture':
                p['capture_id'] = 'foreign-pool'
            elif label == 'foreign-query':
                p['query_capture_id'] = 'foreign-query'
            elif label == 'stale-observer':
                q['task_bindings']['pool']['observer'] += 1
            else:
                row = next(e for e in q['task_bindings']['events'] if e['target'] is not None)
                if label == 'outside-interval':
                    row['target']['at_pool_ns'] = max(e['at_ns'] for e in pool['events']) + 1
                else:
                    row['target']['task'] = pool['task_allocator_high_water'] + 1
            if label != 'missing-pool':
                (altered / pool_path.name).write_text(json.dumps(p))
            (altered / query_path.name).write_text(json.dumps(q))
            coverage = self.assess(altered, altered / manifest_path.name)
            self.assertFalse(coverage['task_bindings'][query['capture_id']]['graph_binding_valid'], coverage)
            self.assertFalse(coverage['task_bindings'][query['capture_id']]['graph_binding_complete'], coverage)
            # Pool mapping cannot erase independently valid compiler launch evidence.
            for key in ('structural_complete', 'expected_launches', 'observed_entries', 'observed_queries'):
                self.assertEqual(coverage[key], baseline[key], (label, key, coverage))

        legacy = self.root / 'selected-legacy3'
        shutil.copytree(sink, legacy)
        for path in legacy.glob('*.json'):
            receipt = json.loads(path.read_text())
            if receipt.get('producer') == 'rustc-query-admission':
                receipt['schema'] = 3
                receipt.pop('task_bindings')
            elif receipt.get('producer') == 'rustc-query-admission-entry':
                receipt['query_receipt_schema'] = 3
            elif receipt.get('producer') == 'rustc-thread-pool-join':
                receipt['schema'] = 1
                receipt.pop('query_capture_id')
            path.write_text(json.dumps(receipt))
        coverage = self.assess(legacy, legacy / manifest_path.name)
        self.assertEqual(coverage['structural_complete'], baseline['structural_complete'])
        self.assertNotIn('task_bindings', coverage, 'legacy absence is unknown, not complete or zero')

    def test_actual_binding_neutrality_failure_and_missing_evidence(self):
        self.build()
        self.run_command([self.binary])
        # Establish a byte-stable warm recompilation baseline before capture.
        self.force_same_source_rebuild()
        self.build()
        before = self.digest()
        self.force_same_source_rebuild()
        self.build()
        self.assertEqual(before, self.digest(), 'capture-off baseline is not byte-stable')

        captured = self.private_sink('bound')
        self.force_same_source_rebuild()
        self.build(captured)
        bound = self.inspect_bound(captured)
        self.assertEqual(before, self.digest(), 'capture-on executable bytes changed')
        self.run_command([self.binary])
        self.force_same_source_rebuild()
        self.build()
        self.assertEqual(before, self.digest(), 'capture-off executable bytes changed')

        fresh = self.private_sink('fresh')
        self.build(fresh)
        self.assertEqual(before, self.digest())
        for path, manifest in self.manifests(fresh):
            coverage = self.assess(fresh, path, complete=True)
            self.assertGreater(manifest['expected_units'], 0)
            self.assertEqual(manifest['expected_launches'], 0)
            self.assertEqual(manifest['launches'], [])
            self.assertEqual(coverage['observed_entries'], 0)
            self.assertEqual(coverage['observed_queries'], 0)
            self.assertTrue(all(unit['disposition'] == 'fresh' for unit in manifest['units']))

        self.assertFalse(any(json.loads(path.read_text()).get('producer') in
                             ('rustc-query-admission-entry', 'rustc-query-admission')
                             for path in fresh.glob('*.json')),
                         'fresh Cargo units must not manufacture compiler evidence')

        # Remove real evidence one kind at a time, restoring exact bytes afterwards.
        # The independent initial declaration may cause a hard error, never success.
        manifest_path, manifest, launch = bound[0]
        authority = manifest['authority_root']
        journal = next(captured.glob(f'compiler-owner-{authority}.event-*.json'))
        victims = [captured / (launch['entry_capture_id'] + '.json'),
                   captured / (launch['capture_id'] + '.json'), journal,
                   captured / f'compiler-owner-{authority}.declaration.json']
        for victim in victims:
            with self.subTest(missing=victim.name):
                saved = victim.read_bytes()
                victim.unlink()
                try:
                    self.assess(captured, manifest_path, complete=False, may_reject=True)
                finally:
                    victim.write_bytes(saved)
        self.assess(captured, manifest_path, complete=True)

        self.source.write_text('fn main() { let _: u32 = "intentional compiler failure"; }\n')
        failed = self.private_sink('failed')
        result = self.build(failed, success=False)
        self.assertIn('mismatched types', result.stdout + result.stderr)
        failed_launches = self.inspect_bound(failed, complete=False)
        self.assertTrue(any(launch['outcome'] == 'failure' and launch['wait_status'] != 0
                            for _, _, launch in failed_launches))

        self.source.write_text(self.original)
        self.build()
        self.force_same_source_rebuild()
        self.build()
        restored = self.digest()
        blocked = self.root / 'not-a-directory'
        blocked.write_text('preserve existing private data')
        self.force_same_source_rebuild()
        self.build(blocked)
        self.assertEqual(blocked.read_text(), 'preserve existing private data')
        self.assertEqual(restored, self.digest())
        nonprivate = self.root / 'nonprivate'
        nonprivate.mkdir(mode=0o755)
        nonprivate.chmod(0o755)
        self.force_same_source_rebuild()
        self.build(nonprivate)
        self.assertEqual(nonprivate.stat().st_mode & 0o777, 0o755)
        self.assertEqual(restored, self.digest())
        self.assertFalse(any(json.loads(path.read_text()).get('producer') ==
                             'rustc-query-admission-entry' for path in nonprivate.glob('*.json')),
                         'nonprivate sink must not admit compiler binding')
        self.run_command([self.binary])
        print('native compiler binding acceptance passed: selected structural coverage only, '
              'schema4, warm off/on/off byte identity, fresh zero launches, failure, '
              'missing evidence and fail-open sinks', flush=True)


if __name__ == '__main__':
    unittest.main()
