"""Mocked orchestration only. Never invokes a native engine or compiler."""
import argparse
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import resident_engine_compare as compare


def output(failing=False, marker=None, names=None, filtered=144):
    names = compare.NAMES if names is None else names
    lines = [f'test {name} ... {"FAILED" if failing and name == compare.EDIT_TEST else "ok"}'
             for name in names]
    if marker:
        lines.append(marker)
    if failing:
        lines.append('JR_RUNTIME_FAILURE')
    lines.append(f'test result: {"FAILED" if failing else "ok"}. '
                 f'{len(names) - int(failing)} passed; {int(failing)} failed; '
                 f'0 ignored; 0 measured; {filtered} filtered out')
    return '\n'.join(lines) + '\n'


class HarnessTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='rec-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        source = self.root / 's'
        (source / 'src').mkdir(parents=True)
        self.original = b'fn shares_overlapping_time() {\n}\n'
        (source / 'src/summary.rs').write_bytes(self.original)
        for name in ('ra-jit', 'jr-engine', 'blaze'):
            (self.root / name).write_bytes(b'\x7fELFmock')
        self.args = argparse.Namespace(source=source, output=self.root / 'o',
            reference_engine=self.root / 'ra-jit', engine=self.root / 'jr-engine',
            blaze=None, samples=5, timeout=1)
        self.calls = []
        self.fault = None

    def fake_run(self, cmd, cwd, env, prefix, timeout):
        stack = next(s for s in ('installed-blaze', 'reference', 'engine')
                     if prefix.name.startswith(s + '-'))
        phase, rep = prefix.name[len(stack) + 1:].rsplit('-', 1)
        self.calls.append((stack, phase, int(rep), list(cmd), dict(env)))
        if self.fault:
            changed = self.fault(stack, phase, int(rep))
            if isinstance(changed, BaseException):
                raise changed
        else:
            changed = None
        status = 0
        if phase == 'version':
            text = (compare.REFERENCE_VERSION + '\n' if stack == 'reference' else
                    ('jr-engine' if stack == 'engine' else 'blaze') + ' 0.1.0 source-built\n')
        elif phase == 'type-error':
            text, status = 'error[E0308]: JR_RUNTIME_TYPE_ERROR\n', 101
        elif phase == 'assert-failure':
            text, status = output(failing=True), 101
        else:
            text = output(marker=f'JR_RUNTIME_EXECUTED_{int(rep) + 1}' if phase == 'edit' else None)
        if changed is not None:
            text, status = changed
        prefix.with_suffix('.stdout').write_text(text)
        prefix.with_suffix('.stderr').write_text('')
        return dict(status=status, error=None, timed_out=False, elapsed_seconds=0.01,
                    command=cmd)

    def run_harness(self):
        with patch.object(compare, 'pinned_source'), patch.object(compare.runtime, 'run_process',
                                                                  side_effect=self.fake_run):
            return compare.execute(self.args)

    def summary(self):
        return json.loads((self.args.output / 'summary.json').read_text())

    def test_full_gates_rotation_edits_and_direct_commands(self):
        self.assertEqual(self.run_harness(), 0)
        self.assertTrue(self.summary()['source_built_parity'])
        edits = [(s, n) for s, phase, n, _, _ in self.calls if phase == 'edit']
        self.assertEqual(edits, [(s, n) for n in range(5) for s in
                                compare.runtime.order(['reference', 'engine'], n)])
        self.assertEqual(len(self.calls), 22)
        for stack, phase, _, cmd, env in self.calls:
            if phase != 'version':
                self.assertEqual(cmd[1:], (['cargo'] if stack == 'reference' else []) +
                                 ['test'] + compare.FILTER)
            self.assertEqual(env.get('HOME'), os.environ.get('HOME'))
            self.assertTrue(Path(env['BLAZE_CACHE']).is_relative_to(self.args.output / stack))
            self.assertEqual('JR_ENGINE_HOME' in env, stack == 'engine')
        self.assertEqual((self.args.source / 'src/summary.rs').read_bytes(), self.original)
        metadata = json.loads((self.args.output / 'metadata.json').read_text())
        self.assertEqual(metadata['source_before'], metadata['source_after'])
        self.assertEqual(metadata['tools_before'], metadata['tools_after'])

    def test_zero_tests_never_pass(self):
        self.fault = lambda s, p, n: (output(names=[]), 0) if p == 'prepare' else None
        self.assertEqual(self.run_harness(), 1)
        self.assertEqual(self.summary()['medians_seconds'], {})
        self.assertFalse(any(p == 'edit' for _, p, _, _, _ in self.calls))

    def test_stale_duplicate_or_wrong_marker_excludes_stack(self):
        for marker in ('JR_RUNTIME_EXECUTED_99', 'JR_RUNTIME_EXECUTED_1\nJR_RUNTIME_EXECUTED_1',
                       'JR_RUNTIME_EXECUTED_1suffix', ''):
            with self.subTest(marker=marker):
                self.args.output = self.root / ('o' + str(len(list(self.root.glob('o*')))))
                self.fault = lambda s, p, n: (output(marker=marker), 0) if s == 'engine' and p == 'edit' and n == 0 else None
                self.assertEqual(self.run_harness(), 1)
                self.assertEqual(self.summary()['excluded'], ['engine'])

    def test_wrong_names_or_filtered_counts_excluded(self):
        self.fault = lambda s, p, n: (output(filtered=145), 0) if s == 'engine' and p == 'edit' else None
        self.assertEqual(self.run_harness(), 1)
        parsed = compare.runtime.test_results(output(names=compare.NAMES[:-1] + ['wrong']))
        self.assertFalse(compare.valid_tests(parsed))

    def test_failing_assert_gate_must_actually_fail(self):
        self.fault = lambda s, p, n: (output(), 0) if p == 'assert-failure' else None
        self.assertEqual(self.run_harness(), 1)
        self.assertEqual(self.summary()['medians_seconds'], {})

    def test_type_gate_requires_diagnostic_and_recovery(self):
        self.fault = lambda s, p, n: ('JR_RUNTIME_TYPE_ERROR', 101) if p == 'type-error' else None
        self.assertEqual(self.run_harness(), 1)
        self.assertEqual(self.summary()['medians_seconds'], {})

    def test_cold_failure_not_replaced_or_seeded(self):
        self.fault = lambda s, p, n: ('unsupported writable static', 1) if p == 'prepare' else None
        with patch.object(compare.runtime, 'seed_cache') as seed:
            self.assertEqual(self.run_harness(), 1)
            seed.assert_not_called()
        metadata = json.loads((self.args.output / 'metadata.json').read_text())
        self.assertEqual(metadata['status'], 'failed/incomplete')

    def test_restore_on_exception(self):
        self.fault = lambda s, p, n: RuntimeError('interrupted') if p == 'type-error' else None
        with self.assertRaisesRegex(RuntimeError, 'interrupted'):
            self.run_harness()
        self.assertEqual((self.args.source / 'src/summary.rs').read_bytes(), self.original)
        self.assertEqual(self.summary()['medians_seconds'], {})

    def test_tool_mutation_invalidates_all(self):
        def mutate(s, p, n):
            if s == 'engine' and p == 'edit' and n == 4:
                self.args.engine.write_bytes(b'\x7fELFchanged')
        self.fault = mutate
        self.assertEqual(self.run_harness(), 1)
        self.assertEqual(self.summary()['medians_seconds'], {})

    def test_native_identity_required(self):
        self.fault = lambda s, p, n: ('blaze 1.0', 0) if s == 'engine' and p == 'version' else None
        self.assertEqual(self.run_harness(), 1)
        self.assertIn('engine', self.summary()['excluded'])

    def test_reference_exact_origin_required(self):
        self.fault = lambda s, p, n: ('blaze 0.20261027.0 (other-origin)', 0) if s == 'reference' and p == 'version' else None
        self.assertEqual(self.run_harness(), 1)
        self.assertFalse(self.summary()['source_built_parity'])
        self.assertEqual(self.summary()['medians_seconds'], {})

    def test_failed_recovery_not_accepted(self):
        self.fault = lambda s, p, n: ('recovery compile failure', 101) if p == 'assert-recovery' else None
        self.assertEqual(self.run_harness(), 1)
        self.assertEqual(self.summary()['medians_seconds'], {})

    def test_missing_tool_at_final_hash_fails_closed(self):
        def remove(s, p, n):
            if s == 'engine' and p == 'edit' and n == 4:
                self.args.engine.unlink()
        self.fault = remove
        self.assertEqual(self.run_harness(), 1)
        self.assertEqual(self.summary()['medians_seconds'], {})

    def test_source_special_inputs_rejected(self):
        linked = self.args.source / 'linked'
        linked.symlink_to(self.args.source / 'src/summary.rs')
        with self.assertRaisesRegex(ValueError, 'links'):
            compare.source_hashes(self.args.source)

    def test_native_wrapper_rejected_before_invocation(self):
        self.args.engine.write_bytes(b'#!/bin/sh\nexec blaze "$@"\n')
        with self.assertRaisesRegex(ValueError, 'native ELF'):
            self.run_harness()
        self.assertEqual(self.calls, [])

    def test_installed_blaze_separate(self):
        self.args.blaze = self.root / 'blaze'
        self.assertEqual(self.run_harness(), 0)
        self.assertTrue(self.summary()['source_built_parity'])
        self.assertEqual(len(self.summary()['medians_seconds']), 3)

    def test_engine_operations_are_direct(self):
        for operation in ('test', 'build', 'check', 'run'):
            self.assertEqual(compare.command('/native/jr-engine', 'engine', operation, ['--lib']),
                             ['/native/jr-engine', operation, '--lib'])

    def test_env_overrides_removed_and_private(self):
        with patch.dict(os.environ, {'JR_ENGINE_HOME': '/production', 'BLAZE_CACHE': '/production',
                                     'RUSTFLAGS': 'bad', 'CARGO_ENCODED_RUSTFLAGS': 'bad'}):
            env = compare.environment(self.args.output, 'engine')
        self.assertNotIn('RUSTFLAGS', env)
        self.assertNotIn('CARGO_ENCODED_RUSTFLAGS', env)
        self.assertEqual(env['CARGO_PROFILE_TEST_OPT_LEVEL'], '0')
        self.assertEqual(env['BLAZE_BACKEND'], 'cranelift')
        self.assertTrue(Path(env['JR_ENGINE_HOME']).is_relative_to(self.args.output))

    def test_pin_verifies_all_blobs_without_workload(self):
        data = self.original
        import hashlib
        oid = hashlib.sha1(b'blob ' + str(len(data)).encode() + b'\0' + data).hexdigest()
        tree = f'100644 blob {oid}\tsrc/summary.rs\0'.encode()
        with patch.object(compare.subprocess, 'check_output', return_value=tree):
            compare.pinned_source(self.args.source)
            (self.args.source / 'src/summary.rs').write_bytes(data + b'changed')
            with self.assertRaisesRegex(ValueError, 'does not match'):
                compare.pinned_source(self.args.source)

    def test_fresh_short_output_and_samples_required(self):
        self.args.samples = 4
        with self.assertRaises(ValueError):
            self.run_harness()
        self.args.samples = 5
        self.args.output.mkdir()
        with self.assertRaises(ValueError):
            self.run_harness()


if __name__ == '__main__':
    unittest.main()
