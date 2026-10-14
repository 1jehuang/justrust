#!/usr/bin/env python3
"""Opt-in acceptance against the rebuilt native compiler, not a fleet binary.

JR_QUERY_RUSTC selects the actual rebuilt stage1 rustc. The standalone native
UNTRACKED flag is not injected through Cargo and proves no Cargo key neutrality.
"""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

from test_execution_graph_task_binding_native import assert_binding_inventory


@unittest.skipUnless(os.environ.get('JR_QUERY_RUSTC'),
                     'requires an actual rebuilt native query producer')
class NativeQueryAdmissionAcceptance(unittest.TestCase):
    def test_actual_admission_counts_artifact_reuse_and_fail_open(self):
        evidence = os.environ.get('JR_QUERY_EVIDENCE_DIR')
        if evidence:
            Path(evidence).mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(prefix='jr-query-acceptance-',
                                         dir=os.environ.get('JCODE_SCRATCH_DIR')) as tmp:
            root = (Path(tempfile.mkdtemp(prefix='admission-', dir=evidence))
                    if evidence else Path(tmp))
            print(json.dumps({'evidence_dir': str(root), 'retained': bool(evidence)}), flush=True)
            serial = 0
            rustc = str(Path(os.environ['JR_QUERY_RUSTC']).resolve())
            justrust = os.environ.get('JR_QUERY_JUSTRUST', shutil.which('justrust'))
            self.assertIsNotNone(justrust)
            env = os.environ.copy()
            env.pop('JUSTRUST_EXECUTION_GRAPH_DIR', None)
            source = root / 'fixture.rs'
            original = ('const _: () = assert!(option_env!("JUSTRUST_EXECUTION_GRAPH_DIR").is_none());\n'
                        'pub fn sum(n: u32) -> u32 { (0..n).fold(0u32, |a,b| a.wrapping_add(b)) }\n'
                        'fn main() { assert_eq!(sum(10), 45); }\n')
            source.write_text(original)
            artifact = root / 'fixture-bin'
            args = [rustc, '--crate-name=query_admission_fixture', '--crate-type=bin',
                    '--edition=2024', '-Copt-level=0', '-Cdebuginfo=0',
                    '-Cincremental=' + str(root / 'incremental'),
                    '-Zincremental-info', '-o', str(artifact), str(source)]

            def compile(capture=None, parallel=False, success=True):
                nonlocal serial
                serial += 1
                command = args.copy()
                if capture is not None:
                    command.insert(1, '-Zjr-query-graph=' + str(capture))
                if parallel:
                    command.insert(1, '-Zthreads=2')
                p = subprocess.run(command, cwd=root, env=env, capture_output=True,
                                   text=True, timeout=180)
                output = p.stdout + p.stderr
                (root / f'compile-{serial}.json').write_text(json.dumps({
                    'command': command, 'exit_code': p.returncode, 'output': output,
                    'source_sha256': hashlib.sha256(source.read_bytes()).hexdigest()}, indent=2))
                shutil.copyfile(source, root / f'source-{serial}.rs')
                if success:
                    self.assertEqual(p.returncode, 0, output)
                else:
                    self.assertNotEqual(p.returncode, 0, output)
                return output

            def inspect(capture, complete=True, parallel=False):
                p = subprocess.run([justrust, 'graph', str(capture), '--json'],
                                   cwd=root, env=env, capture_output=True, text=True,
                                   timeout=30)
                self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
                assessments = json.loads(p.stdout)
                self.assertTrue(assessments)
                producers = {'rustc-query-admission', 'rustc-thread-pool-join'}
                self.assertTrue(all(a['producer'] in producers for a in assessments), p.stdout)
                (root / f'{capture.name}-assessment.json').write_text(json.dumps({
                    'stdout': p.stdout, 'stderr': p.stderr, 'exit_code': p.returncode}, indent=2))
                self.assertTrue(all(not a['full_pipeline_complete'] for a in assessments))
                self.assertTrue(all(a['cpu_work_ns'] is None and a['execution_span_ns'] is None
                                    for a in assessments))
                query_assessments = [a for a in assessments
                                     if a['producer'] == 'rustc-query-admission']
                pool_assessments = [a for a in assessments
                                    if a['producer'] == 'rustc-thread-pool-join']
                self.assertTrue(query_assessments, p.stdout)
                if complete:
                    self.assertTrue(all(a['structural_complete'] for a in assessments), p.stdout)
                else:
                    self.assertTrue(all(not a['structural_complete']
                                        for a in query_assessments), p.stdout)
                all_receipts = [json.loads(p.read_text()) for p in capture.glob('*.json')]
                for receipt in all_receipts:
                    self.assertIn(receipt['producer'], producers, receipt)
                receipts = [r for r in all_receipts if r['producer'] == 'rustc-query-admission']
                pools = [r for r in all_receipts if r['producer'] == 'rustc-thread-pool-join']
                self.assertTrue(receipts, 'query receipts required')
                if parallel:
                    self.assertTrue(pools, 'parallel capture requires separate pool receipts')
                self.assertEqual(len(receipts), len(query_assessments))
                self.assertEqual(len(pools), len(pool_assessments))
                for pool in pools:
                    self.assertEqual(pool['schema'], 2)
                    self.assertIn(pool['query_capture_id'], {r['capture_id'] for r in receipts})
                    self.assertFalse(pool['full_pipeline_complete'])
                    self.assertIsNone(pool['cpu_work_ns'])
                    self.assertIsNone(pool['execution_span_ns'])
                    if complete:
                        self.assertTrue(pool['sealed'])
                        self.assertEqual(pool['interface_outcome'], 'returned')
                for receipt in receipts:
                    assert_binding_inventory(self, receipt)
                    self.assertEqual(receipt['scope'], 'query-job-admission-lifecycle')
                    self.assertFalse(receipt['full_pipeline_complete'])
                    if complete:
                        self.assertEqual(receipt['expected_contexts'], len(receipt['contexts']))
                        self.assertEqual(sorted(c['context_id'] for c in receipt['contexts']),
                                         list(range(1, receipt['expected_contexts'] + 1)))
                    jobs = {}
                    terminals = {}
                    for event in receipt['events']:
                        key = (event['context_id'], event['job_id'])
                        if event['kind'] == 'admitted':
                            self.assertNotIn(key, jobs)
                            jobs[key] = event
                        else:
                            self.assertNotIn(key, terminals)
                            terminals[key] = event
                    self.assertEqual(set(jobs), set(terminals))
                    for context in receipt['contexts']:
                        if context['allocator_high_water'] is not None:
                            self.assertEqual(context['allocator_baseline'], 1)
                            expected = context['allocator_high_water'] - 1
                            self.assertEqual(context['expected_jobs'], expected)
                            observed = sum(key[0] == context['context_id'] for key in jobs)
                            self.assertEqual(observed, expected)
                return receipts

            compile()
            subprocess.run([str(artifact)], check=True, cwd=root, env=env, timeout=30)
            # Compare equally warm compilations, not cold output versus incremental output.
            compile()
            before = hashlib.sha256(artifact.read_bytes()).hexdigest()
            compile()
            self.assertEqual(before, hashlib.sha256(artifact.read_bytes()).hexdigest(),
                             'capture-off baseline itself is not byte-stable')
            capture = root / 'incremental-capture'
            compile(capture)
            inspect(capture)
            self.assertEqual(before, hashlib.sha256(artifact.read_bytes()).hexdigest())
            compile()
            self.assertEqual(before, hashlib.sha256(artifact.read_bytes()).hexdigest())

            # A separate incremental directory exercises non-warm admissions.
            args[args.index('-Cincremental=' + str(root / 'incremental'))] = (
                '-Cincremental=' + str(root / 'parallel-incremental'))
            parallel = root / 'parallel-capture'
            compile(parallel, parallel=True)
            receipts = inspect(parallel, parallel=True)
            self.assertTrue(any(e['kind'] == 'admitted' for r in receipts for e in r['events']))

            source.write_text('pub fn bad() -> u32 { "intentional query type error" }\nfn main() {}\n')
            failed = root / 'failed-capture'
            output = compile(failed, success=False)
            self.assertIn('mismatched types', output)
            # Fatal interface unwinding must not seal a complete query capture.
            inspect(failed, complete=False)

            source.write_text(original)
            blocked = root / 'not-a-directory'
            blocked.write_text('existing private data')
            compile(blocked)
            self.assertEqual(blocked.read_text(), 'existing private data')
            print('native query admission acceptance: allocator accounting, serial/parallel, '
                  'artifact identity, failed interface and fail-open sink passed')


if __name__ == '__main__':
    unittest.main()
