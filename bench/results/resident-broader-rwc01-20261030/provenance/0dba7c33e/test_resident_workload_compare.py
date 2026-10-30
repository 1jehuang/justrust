#!/usr/bin/env python3
"""Source-only fixture checks and mocked receipts. Never run native engines."""
import argparse
import copy
import json
from pathlib import Path
import re
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import resident_workload_compare as harness

REPO = Path(__file__).resolve().parent.parent


def pinned(path):
    return subprocess.check_output(['git', 'show', f'f97fcc2:{path}'], cwd=REPO, timeout=10)


def transcript(names, failed=None, filtered=200):
    lines = [f'test {n} ... {"FAILED" if n == failed else "ok"}' for n in names]
    lines.append(f'test result: {"FAILED" if failed else "ok"}. '
                 f'{len(names) - bool(failed)} passed; {int(bool(failed))} failed; '
                 f'0 ignored; 0 measured; {filtered} filtered out; finished in 0.00s')
    return '\n'.join(lines)


class FixtureTests(unittest.TestCase):
    def test_exact_pinned_anchors_and_original_function(self):
        self.assertGreaterEqual(len({x[0] for x in harness.WORKLOADS.values()}), 3)
        for name, (path, prefix, anchor, test, kind) in harness.WORKLOADS.items():
            source = pinned(path)
            self.assertEqual(source.count(anchor.encode()), 1)
            self.assertIn(('fn ' + test + '() {').encode(), source)
            for phase in ('type-error', 'assert-failure', 'edit'):
                changed = harness.edited(source, name, phase, 2)
                self.assertNotEqual(changed, source)
                self.assertEqual(changed.count(anchor.encode()), 1)
            self.assertIn(b'JR_WORKLOAD_EXECUTED_', harness.edited(source, name, 'edit'))
            if kind == 'ordinary':
                before = source.split(anchor.encode())[0]
                self.assertFalse(before.rstrip().endswith(b'#[test]'))
                self.assertIn(b'let p = parse_pass(', source)
                self.assertEqual(prefix, 'shim::tests::')

    def test_reject_ambiguous_anchor(self):
        anchor = harness.WORKLOADS['shim-test'][2].encode()
        for source in (b'', anchor + anchor):
            with self.assertRaises(ValueError):
                harness.edited(source, 'shim-test', 'edit')

    def test_exact_contract(self):
        names = ['shim::tests::describes_cargo_rustc_args', 'shim::tests::parses_time_passes_json']
        expected = harness.runtime.test_results(transcript(names))
        self.assertEqual(harness.discover(expected, 'shim-test'), expected)
        self.assertTrue(harness.valid(expected, expected))
        for field in ('filtered', 'passed', 'ignored'):
            bad = copy.deepcopy(expected)
            bad[field] += 1
            self.assertFalse(harness.valid(bad, expected))
        bad = copy.deepcopy(expected)
        bad['tests'].append(('shim::tests::unexpected', 'ok'))
        self.assertFalse(harness.valid(bad, expected))
        self.assertIsNone(harness.discover(None, 'shim-test'))


class MockWorkflowTests(unittest.TestCase):
    def workflow(self, mode='ok', samples=2, selection=None):
        with tempfile.TemporaryDirectory(prefix='jrwc-') as temp:
            root = Path(temp)
            source = root / 'source63'
            (source / 'src').mkdir(parents=True)
            originals = {x[0]: pinned(x[0]) for x in harness.WORKLOADS.values()}
            for path, data in originals.items():
                (source / path).write_bytes(data)
            tools = []
            for name in ('reference', 'engine'):
                tool = root / name
                tool.write_bytes(b'\x7fELFmock ' + name.encode())
                tool.chmod(0o700)
                tools.append(tool)
            toolchain = root / harness.shared.TOOLCHAIN_NAME
            toolchain.mkdir()
            args = argparse.Namespace(source=source, output=root / 'out', toolchain=toolchain,
                                      reference_engine=tools[0], engine=tools[1],
                                      proc_macro_server=None, samples=samples, timeout=3,
                                      workload=selection)
            calls = []

            def run(command, cwd, env, prefix, timeout):
                self.assertEqual(timeout, 3)
                self.assertEqual(cwd, source)
                stack = 'reference' if command[0] == str(tools[0]) else 'engine'
                phase = next((p for p in ('version', 'prepare', 'type-error', 'type-recovery',
                                         'assert-failure', 'assert-recovery', 'edit')
                              if '-' + p + '-' in prefix.name), None)
                workload = next(w for w in harness.WORKLOADS if '-' + w + '-' in prefix.name)
                path, module, _, failing, _ = harness.WORKLOADS[workload]
                contents = (source / path).read_bytes()
                calls.append((stack, workload, phase, contents))
                if mode == 'exception' and phase == 'edit':
                    raise RuntimeError('synthetic interruption')
                names = sorted(module + n.decode() for n in re.findall(
                    rb'#\[test\]\s+fn (\w+)\(', originals[path]))
                status, error, timed_out = 0, None, False
                if phase == 'version':
                    text = harness.shared.REFERENCE_VERSION if stack == 'reference' else 'jr-engine mock'
                elif phase == 'type-error':
                    status, text = 101, 'error[E0308]: JR_WORKLOAD_TYPE_ERROR'
                    if mode == 'type-ignored' and stack == 'engine':
                        status, text = 0, transcript(names)
                elif phase == 'assert-failure':
                    status, text = 101, transcript(names, module + failing) + '\nJR_WORKLOAD_FAILURE'
                else:
                    text = transcript(names)
                    if phase == 'edit':
                        marker = re.search(rb'JR_WORKLOAD_EXECUTED_[^"\s]+', contents).group().decode()
                        if mode == 'stale-marker' and stack == 'engine':
                            marker += '_stale'
                        text += '\n' + marker
                    if mode == 'wrong-count' and stack == 'engine':
                        text = text.replace('200 filtered', '201 filtered')
                    if mode == 'timeout' and stack == 'engine' and phase == 'edit':
                        timed_out, status = True, -9
                    if mode == 'identity' and phase == 'edit':
                        tools[1].write_bytes(b'\x7fELFchanged')
                if mode == 'bad-assert' and stack == 'engine' and phase == 'assert-failure':
                    status, text = 0, transcript(names)
                if mode == 'bad-recovery' and stack == 'engine' and phase == 'assert-recovery':
                    status = 101
                if mode == 'reference-failed' and stack == 'reference' and phase == 'prepare':
                    status = 101
                if mode == 'source-mutation' and stack == 'engine' and phase == 'edit':
                    (source / path).write_bytes(contents + b'\n// mutated\n')
                prefix.with_suffix('.stdout').write_text(text)
                prefix.with_suffix('.stderr').write_text('')
                return dict(command=command, status=status, error=error, timed_out=timed_out,
                            elapsed_seconds=0.125)

            with patch.object(harness.shared, 'pinned_source') as pristine, patch.object(
                    harness.shared, 'toolchain_identity', return_value={'frozen': 'mock'}), patch.object(
                    harness.runtime, 'run_process', side_effect=run):
                if mode == 'exception':
                    with self.assertRaisesRegex(RuntimeError, 'synthetic interruption'):
                        harness.execute(args)
                    result = None
                else:
                    result = harness.execute(args)
                pristine.assert_called_once_with(source)
            for path, data in originals.items():
                self.assertEqual((source / path).read_bytes(), data)
            metadata = json.loads((args.output / 'metadata.json').read_text())
            summary = json.loads((args.output / 'summary.json').read_text())
            records = json.loads((args.output / 'records.json').read_text())
            for row in records:
                self.assertTrue(Path(row['raw_prefix'] + '.stdout').exists())
            return result, metadata, summary, calls

    def test_complete_rotation_identical_bytes_and_positive_samples(self):
        for samples in (1, 3, 5):
            result, metadata, summary, calls = self.workflow(samples=samples)
            self.assertEqual(result, 0)
            self.assertEqual(metadata['status'], 'complete')
            self.assertEqual(len(metadata['contracts']), 4)
            for workload in harness.WORKLOADS:
                edits = [c for c in calls if c[1] == workload and c[2] == 'edit']
                self.assertEqual(len(edits), samples * 2)
                for rep in range(samples):
                    a, b = edits[rep * 2:rep * 2 + 2]
                    self.assertEqual(a[3], b[3])
                    self.assertEqual([a[0], b[0]], harness.runtime.order(harness.STACKS, rep))
            self.assertTrue(all(not x for x in summary['excluded'].values()))

    def test_failed_stacks_excluded(self):
        for mode in ('type-ignored', 'wrong-count', 'stale-marker', 'timeout',
                     'bad-assert', 'bad-recovery', 'source-mutation'):
            result, metadata, summary, _ = self.workflow(mode)
            self.assertEqual(result, 1, mode)
            self.assertEqual(metadata['status'], 'failed/incomplete')
            self.assertTrue(all('engine' in x for x in summary['excluded'].values()), mode)

    def test_missing_reference_contract_cannot_qualify_candidate(self):
        result, _, summary, calls = self.workflow('reference-failed')
        self.assertEqual(result, 1)
        self.assertTrue(all(set(x) == set(harness.STACKS) for x in summary['excluded'].values()))
        self.assertFalse(any(c[2] == 'edit' for c in calls))

    def test_mutated_identity_excludes_everything(self):
        result, metadata, summary, _ = self.workflow('identity')
        self.assertEqual(result, 1)
        self.assertFalse(metadata['unchanged'])
        self.assertEqual(summary['medians_seconds'], {})

    def test_interruption_restores_backup(self):
        _, metadata, summary, _ = self.workflow('exception')
        self.assertIn('error', metadata)
        self.assertEqual(summary['medians_seconds'], {})

    def test_selection(self):
        result, metadata, _, calls = self.workflow(selection=['shim-function'])
        self.assertEqual(result, 0)
        self.assertEqual(list(metadata['contracts']), ['shim-function'])
        self.assertEqual({c[1] for c in calls}, {'shim-function'})

    def test_validation_before_source_access(self):
        for samples, timeout in ((0, 1), (-1, 1), (1, 0), (1, float('nan')), (1, float('inf'))):
            args = argparse.Namespace(samples=samples, timeout=timeout)
            with self.assertRaises(ValueError):
                harness.execute(args)


if __name__ == '__main__':
    unittest.main()
