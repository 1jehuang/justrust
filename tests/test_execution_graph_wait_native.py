#!/usr/bin/env python3
"""Real native query-wait acceptance. Only an unset JR_QUERY_RUSTC skips.

Run after rebuilding stage1 and the graph consumer, for example:
JR_QUERY_RUSTC=/path/to/stage1/bin/rustc python3 tests/test_execution_graph_wait_native.py
Evidence is printed as JSON, including actual observed cycle/poison counts.
Set JR_QUERY_EVIDENCE_DIR to retain fixtures, binaries, receipts and compile logs.
"""
import collections
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time
import unittest


def shared_query_fixture(functions=512, width=512, depth=64):
    """Independent typecheck jobs converge on one costly projection-bearing alias."""
    lines = ['#![allow(dead_code)]',
             'const _: () = assert!(option_env!("JUSTRUST_EXECUTION_GRAPH_DIR").is_none());',
             'struct Seed;', 'trait Step0 { type Out; }',
             'impl Step0 for Seed { type Out = u64; }']
    for n in range(1, depth):
        lines += [f'trait Step{n} {{ type Out; }}',
                  f'impl Step{n} for Seed {{ type Out = <Seed as Step{n-1}>::Out; }}']
    projection = f'<Seed as Step{depth-1}>::Out'
    lines.append('type Shared = (' + ','.join([projection] * width) + ',);')
    for n in range(functions):
        lines.append(f'fn independent_{n}(value: &Shared) -> usize {{ '
                     f'let _: &{projection} = &value.{n % width}; '
                     'std::mem::size_of::<Shared>() }')
    lines.append('fn main() { let value: Shared = (' + ','.join(['0u64'] * width) + ',);')
    lines.append('let functions: &[fn(&Shared) -> usize] = &[' +
                 ','.join(f'independent_{n}' for n in range(functions)) + '];')
    lines.append('let sum: usize = functions.iter().map(|f| f(&value)).sum();')
    lines.append('println!("shared-query-bytes={}; independent-sum={}", '
                 'std::mem::size_of::<Shared>(), sum); }')
    return '\n'.join(lines) + '\n'


@unittest.skipUnless(os.environ.get('JR_QUERY_RUSTC'), 'requires JR_QUERY_RUSTC')
class NativeQueryWaitAcceptance(unittest.TestCase):
    def setUp(self):
        evidence = os.environ.get('JR_QUERY_EVIDENCE_DIR')
        if evidence:
            Path(evidence).mkdir(parents=True, exist_ok=True)
            self.root = Path(tempfile.mkdtemp(prefix=self._testMethodName + '-', dir=evidence))
        else:
            self.tmp = tempfile.TemporaryDirectory(prefix='jr-query-waits-',
                                                  dir=os.environ.get('JCODE_SCRATCH_DIR'))
            self.addCleanup(self.tmp.cleanup)
            self.root = Path(self.tmp.name)
        print(json.dumps({'test': self._testMethodName, 'evidence_dir': str(self.root),
                          'retained': bool(evidence)}), flush=True)
        self.rustc = str(Path(os.environ['JR_QUERY_RUSTC']).resolve())
        self.justrust = os.environ.get('JR_QUERY_JUSTRUST') or shutil.which('justrust')
        self.assertTrue(self.justrust, 'justrust consumer is required, not a skip condition')
        self.env = os.environ.copy()
        self.env.pop('JUSTRUST_EXECUTION_GRAPH_DIR', None)
        self.source = self.root / 'fixture.rs'
        self.artifact = self.root / 'fixture-bin'
        self.source.write_text(shared_query_fixture())
        self.serial = 0
        self.capture_threads = {}

    def compile(self, threads, capture=False, incremental=None, success=True):
        self.serial += 1
        directory = self.root / f'receipt-{self.serial}' if capture else None
        command = [self.rustc, '--crate-name=wait_fixture', '--crate-type=bin',
                   '--edition=2024', '-Copt-level=0', '-Cdebuginfo=0',
                   f'-Zthreads={threads}', '-o', str(self.artifact), str(self.source)]
        if incremental is not None:
            command.append('-Cincremental=' + str(incremental))
        if directory is not None:
            command.append('-Zjr-query-graph=' + str(directory))
            self.capture_threads[directory] = threads
        started = time.monotonic()
        result = subprocess.run(command, cwd=self.root, env=self.env,
                                capture_output=True, text=True, timeout=300)
        elapsed = time.monotonic() - started
        output = result.stdout + result.stderr
        evidence = {'command': command, 'exit_code': result.returncode, 'output': output,
                    'elapsed_s': elapsed,
                    'source_sha256': hashlib.sha256(self.source.read_bytes()).hexdigest()}
        if result.returncode == 0:
            evidence['artifact_sha256'] = self.digest()
        (self.root / f'compile-{self.serial}.json').write_text(json.dumps(evidence, indent=2))
        if os.environ.get('JR_QUERY_EVIDENCE_DIR'):
            shutil.copyfile(self.source, self.root / f'source-{self.serial}.rs')
        self.assertEqual(result.returncode == 0, success, output)
        print(json.dumps({'compile': self.serial, 'threads': threads,
                          'capture': capture, 'exit_code': result.returncode,
                          'elapsed_s': elapsed, 'output': output}, sort_keys=True), flush=True)
        return directory, output

    def inspect(self, directory, successful=True):
        files = sorted(directory.glob('*.json'))
        self.assertTrue(files, f'no native receipts in {directory}')
        all_receipts = [json.loads(path.read_text()) for path in files]
        producers = {'rustc-query-admission', 'rustc-thread-pool-join'}
        for receipt in all_receipts:
            self.assertIn(receipt['producer'], producers, receipt)
        receipts = [r for r in all_receipts if r['producer'] == 'rustc-query-admission']
        pools = [r for r in all_receipts if r['producer'] == 'rustc-thread-pool-join']
        self.assertTrue(receipts, 'query receipts must not be silently omitted')
        if self.capture_threads[directory] > 1:
            self.assertTrue(pools, 'parallel capture requires separate pool receipts')
        for pool in pools:
            self.assertFalse(pool['full_pipeline_complete'])
            self.assertIsNone(pool['cpu_work_ns'])
            self.assertIsNone(pool['execution_span_ns'])
            if successful:
                self.assertTrue(pool['sealed'], pool)
                self.assertEqual(pool['interface_outcome'], 'returned')
        counts = collections.Counter()
        reasons = collections.Counter()
        for receipt in receipts:
            self.assertEqual(receipt['schema'], 3, 'requires rebuilt v3 producer')
            self.assertEqual(receipt['producer'], 'rustc-query-admission')
            self.assertFalse(receipt['full_pipeline_complete'])
            self.assertIsNone(receipt['cpu_work_ns'])
            self.assertIsNone(receipt['execution_span_ns'])
            if receipt['schema'] == 3:
                self.assertFalse(receipt['query_reads']['semantic_complete'])
                self.assertFalse(receipt['query_reads']['full_pipeline_complete'])
            events = receipt['wait_events']
            groups = collections.defaultdict(list)
            jobs = {(e['context_id'], e['job_id']) for e in receipt['events']
                    if e['kind'] == 'admitted'}
            for event in events:
                self.assertIn(event['kind'], {'registered', 'notified', 'condvar_returned', 'resumed'})
                groups[event['wait_id']].append(event)
                counts[event['kind']] += 1
                if event['kind'] == 'notified':
                    self.assertIn(event['reason'], {'completed', 'poisoned', 'cycle'})
                    reasons[event['reason']] += 1
                self.assertIn((event['context_id'], event['waitee_job_id']), jobs)
                if event['consumer_job_id'] is not None:
                    self.assertIn((event['context_id'], event['consumer_job_id']), jobs)
            if successful:
                self.assertTrue(receipt['capture_complete'], receipt)
                self.assertTrue(receipt['wait_capture_complete'], receipt)
                self.assertEqual(receipt['lost_events'], 0)
                self.assertEqual(receipt['wait_allocator_baseline'], 1)
                expected = receipt['wait_allocator_high_water'] - 1
                self.assertEqual(receipt['expected_waits'], expected)
                self.assertEqual(sorted(groups), list(range(1, expected + 1)))
                for wait_id, group in groups.items():
                    kinds = collections.Counter(e['kind'] for e in group)
                    for kind in ('registered', 'notified', 'condvar_returned', 'resumed'):
                        self.assertEqual(kinds[kind], 1, (wait_id, group))
                    identities = {(e['context_id'], e['waitee_job_id'], e['consumer_job_id'])
                                  for e in group}
                    self.assertEqual(len(identities), 1, group)
                    stamps = {e['kind']: e['at_ns'] for e in group}
                    self.assertLessEqual(stamps['registered'], stamps['notified'])
                    self.assertLessEqual(stamps['notified'], stamps['condvar_returned'])
                    self.assertLessEqual(stamps['condvar_returned'], stamps['resumed'])
        result = subprocess.run([self.justrust, 'graph', str(directory), '--json'],
                                cwd=self.root, env=self.env, capture_output=True,
                                text=True, timeout=30)
        (self.root / f'{directory.name}-assessment.json').write_text(json.dumps({
            'exit_code': result.returncode, 'stdout': result.stdout,
            'stderr': result.stderr}, indent=2))
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        all_assessments = json.loads(result.stdout)
        self.assertTrue(all_assessments, result.stdout)
        for assessment in all_assessments:
            self.assertIn(assessment['producer'], producers, assessment)
            self.assertFalse(assessment['full_pipeline_complete'])
            self.assertIsNone(assessment['cpu_work_ns'])
            self.assertIsNone(assessment['execution_span_ns'])
        assessments = [a for a in all_assessments
                       if a['producer'] == 'rustc-query-admission']
        pool_assessments = [a for a in all_assessments
                            if a['producer'] == 'rustc-thread-pool-join']
        self.assertEqual(len(assessments), len(receipts), result.stdout)
        self.assertEqual(len(pool_assessments), len(pools), result.stdout)
        self.assertTrue(assessments, 'consumer must assess query receipts')
        if successful:
            self.assertTrue(all(a['structural_complete'] for a in pool_assessments), result.stdout)
            self.assertTrue(all(a['wait_structural_complete'] for a in assessments), result.stdout)
            self.assertEqual(sum(a['waits'] for a in assessments), counts['registered'])
            self.assertEqual(sum(a['control_causal_edges'] for a in assessments), counts['resumed'])
        self.assertTrue(all(not a['full_pipeline_complete'] for a in assessments), result.stdout)
        self.last_receipts = receipts
        self.last_assessments = assessments
        print(json.dumps({'receipts': len(receipts),
                          'receipt_sha256': {p.name: hashlib.sha256(p.read_bytes()).hexdigest()
                                             for p in files},
                          'wait_counts': dict(counts),
                          'notification_reasons': dict(reasons),
                          'consumer': assessments}, sort_keys=True), flush=True)
        return counts, reasons

    def execute(self):
        result = subprocess.run([str(self.artifact)], cwd=self.root, env=self.env,
                                capture_output=True, text=True, timeout=30)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(result.stdout, 'shared-query-bytes=4096; independent-sum=2097152\n')
        return result.stdout

    def digest(self):
        return hashlib.sha256(self.artifact.read_bytes()).hexdigest()

    def test_actual_parallel_waits_and_serial_baseline(self):
        # Fresh nonincremental compilations exercise actual query contention.
        directory, _ = self.compile(1, capture=True)
        counts, _ = self.inspect(directory)
        self.assertEqual(sum(counts.values()), 0, counts)
        self.execute()
        for threads in (4, 8):
            directory, _ = self.compile(threads, capture=True)
            counts, _ = self.inspect(directory)
            for kind in ('registered', 'notified', 'resumed'):
                self.assertGreater(counts[kind], 0,
                                   f'-Zthreads={threads} fixture did not exercise {kind}: {counts}')
            self.execute()

    def test_equally_warmed_capture_identity_and_output(self):
        for threads in (1, 4, 8):
            incremental = self.root / f'incremental-{threads}'
            self.compile(threads, incremental=incremental)
            self.compile(threads, incremental=incremental)
            before = self.digest()
            expected_output = self.execute()
            self.compile(threads, incremental=incremental)
            self.assertEqual(self.digest(), before, 'capture-off baseline is not byte-stable')
            directory, _ = self.compile(threads, capture=True, incremental=incremental)
            self.inspect(directory)
            self.assertEqual(self.digest(), before, 'capture changed equally warm binary')
            self.assertEqual(self.execute(), expected_output)
            self.compile(threads, incremental=incremental)
            self.assertEqual(self.digest(), before)
            self.assertEqual(self.execute(), expected_output)
            print(json.dumps({'threads': threads, 'equally_warmed_sha256': before,
                              'executed_output': expected_output}), flush=True)

    def test_small_incremental_semantic_read_inventory(self):
        # Separate from contention: a bounded fixture can prove read inventory
        # completeness, without claiming the read subsystem is semantically complete.
        self.source.write_text('trait Value { fn value(&self) -> u64; }\n'
                               'impl Value for u64 { fn value(&self) -> u64 { *self } }\n'
                               'fn twice<T: Value>(x: T) -> u64 { x.value() + x.value() }\n'
                               'fn main() { println!("{}", twice(21u64)); }\n')
        directory, _ = self.compile(4, capture=True,
                                    incremental=self.root / 'small-incremental')
        self.inspect(directory)
        tracked = 0
        for receipt in self.last_receipts:
            self.assertEqual(receipt['schema'], 3, 'integrated semantic reads require v3')
            reads = receipt['query_reads']
            self.assertTrue(reads['records_complete'], reads)
            self.assertEqual(reads['lost_events'], 0)
            self.assertFalse(reads['semantic_complete'])
            self.assertFalse(reads['full_pipeline_complete'])
            tracked += sum(e['kind'] == 'dep_read' and e['outcome'] == 'tracked_new'
                           for e in reads['events'])
        self.assertGreater(tracked, 0, 'no actual tracked dependency reads')
        self.assertGreater(sum(a['query_reads']['resolved_job_edges']
                               for a in self.last_assessments), 0,
                           'no resolved native dependency job bindings')
        self.assertTrue(all(a['structural_complete'] for a in self.last_assessments),
                        self.last_assessments)
        result = subprocess.run([str(self.artifact)], cwd=self.root, env=self.env,
                                capture_output=True, text=True, timeout=30)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(result.stdout, '42\n')
        print(json.dumps({'small_incremental_tracked_new_reads': tracked,
                          'executed_output': result.stdout}), flush=True)

    def test_errors_and_semantic_cycles_preserve_diagnostics(self):
        fixtures = [('type_error', 'fn bad() -> u32 { "wrong type" }\nfn main() {}\n',
                     'mismatched types'),
                    ('alias_cycle', 'type First = Second;\ntype Second = First;\n'
                     'fn main() { let _: Option<First> = None; }\n',
                     'cycle detected')]
        for name, source, diagnostic in fixtures:
            self.source.write_text(source)
            _, baseline = self.compile(8, success=False)
            directory, captured = self.compile(8, capture=True, success=False)
            self.assertIn(diagnostic, baseline)
            self.assertEqual(captured, baseline, 'capture changed compiler diagnostics')
            counts, reasons = self.inspect(directory, successful=False)
            # A semantic alias diagnostic does not prove a parallel wait cycle.
            print(json.dumps({'fixture': name, 'actual_wait_counts': dict(counts),
                              'observed_parallel_cycle_notifications': reasons['cycle'],
                              'observed_poison_notifications': reasons['poisoned'],
                              'diagnostics_preserved': True}), flush=True)


if __name__ == '__main__':
    unittest.main(verbosity=2)
