#!/usr/bin/env python3
"""Focused stable-native suite dispatch must not select the unrelated engine."""
import argparse
import io
import json
from pathlib import Path
import sys
import tempfile
import types
import unittest
from unittest.mock import Mock, patch

import suite


class StableNativeDispatchTests(unittest.TestCase):
    def invoke(self, a, b, require_valid=False, allow_compiler_change=False,
               module_name='stable_resident_leaf_suite'):
        module = types.ModuleType(module_name)
        module.compare = Mock()
        with tempfile.TemporaryDirectory() as temporary:
            paths = [Path(temporary) / name for name in ('a.json', 'b.json')]
            for path, result in zip(paths, (a, b)):
                path.write_text(json.dumps(result))
            args = argparse.Namespace(a=str(paths[0]), b=str(paths[1]), floor=10,
                                      require_valid=require_valid,
                                      allow_compiler_change=allow_compiler_change)
            with patch.dict(sys.modules, {module_name: module}):
                suite.cmd_compare(args)
        return module.compare

    def test_exact_stable_native_dispatch(self):
        a = {'schema': 'stable_native_leaf/v1', 'arm': 'control'}
        b = {'schema': 'stable_native_leaf/v1', 'arm': 'leaf'}
        compare = self.invoke(a, b)
        compare.assert_called_once_with(a, b, 10)

    def test_mixed_engine_is_sent_to_fail_closed_stable_validator(self):
        for stable_first in (False, True):
            stable = {'schema': 'stable_native_leaf/v1'}
            unrelated = {'schema': 'native_resident/v1'}
            a, b = (stable, unrelated) if stable_first else (unrelated, stable)
            compare = self.invoke(a, b)
            compare.assert_called_once_with(a, b, 10)

    def test_compiler_optimization_requires_explicit_dispatch(self):
        a = {'schema': 'stable_native_leaf/v1'}
        b = {'schema': 'stable_native_leaf/v1'}
        compare = self.invoke(a, b, allow_compiler_change=True)
        compare.assert_called_once_with(a, b, 10, allow_compiler_change=True)

    def test_arithmetic_dispatch_preserves_optimization_opt_in(self):
        a = {'schema': 'stable_native_arithmetic/v1'}
        b = {'schema': 'stable_native_arithmetic/v1'}
        for changed in (False, True):
            compare = self.invoke(a, b, allow_compiler_change=changed,
                                  module_name='stable_resident_arithmetic_suite')
            if changed:
                compare.assert_called_once_with(a, b, 10, allow_compiler_change=True)
            else:
                compare.assert_called_once_with(a, b, 10)

    def test_mixed_native_workloads_are_rejected_before_dispatch(self):
        for first, second in (('leaf', 'arithmetic'), ('arithmetic', 'leaf'),
                              ('lexer', 'leaf'), ('leaf', 'lexer'),
                              ('lexer', 'arithmetic'), ('arithmetic', 'lexer')):
            with self.assertRaisesRegex(SystemExit, 'different stable native workloads'):
                self.invoke({'schema': f'stable_native_{first}/v1'},
                            {'schema': f'stable_native_{second}/v1'},
                            allow_compiler_change=True)

    def test_arithmetic_cannot_bypass_synthetic_validity_gate(self):
        with self.assertRaisesRegex(SystemExit, 'only synthetic'):
            self.invoke({'schema': 'stable_native_arithmetic/v1'},
                        {'schema': 'stable_native_arithmetic/v1'}, require_valid=True,
                        module_name='stable_resident_arithmetic_suite')

    def test_compiler_change_opt_in_cannot_relax_other_benchmarks(self):
        with self.assertRaisesRegex(SystemExit, 'only stable native leaf'):
            self.invoke({'schema': 'other'}, {'schema': 'other'},
                        allow_compiler_change=True)

    def test_root_cli_uses_shared_fn_callback_contract(self):
        import stable_resident_leaf_suite as adapter
        argv = ['suite.py', 'stable-native-leaf', '--compiler', '/fixture/rustc',
                '--output', '/fixture/output', '--items', '8', '-n', '1']
        with patch.object(adapter, 'run') as run, patch.object(sys, 'argv', argv):
            suite.main()
        run.assert_called_once()
        args = run.call_args.args[0]
        self.assertEqual(args.compiler, Path('/fixture/rustc'))
        self.assertEqual(args.items, 8)
        self.assertEqual(args.repetitions, 1)

    def test_root_arithmetic_cli_requires_hash_attestations_and_uses_fn(self):
        import stable_resident_arithmetic_suite as adapter
        argv = ['suite.py', 'stable-native-arithmetic', '--compiler', '/fixture/rustc',
                '--output', '/fixture/output', '--items', '8', '-n', '1',
                '--expected-driver-sha256', '1' * 64,
                '--expected-compiler-sha256', '2' * 64]
        with patch.object(adapter, 'run') as run, patch.object(sys, 'argv', argv):
            suite.main()
        run.assert_called_once()
        args = run.call_args.args[0]
        self.assertEqual(args.compiler, Path('/fixture/rustc'))
        self.assertEqual(args.items, 8)
        self.assertEqual(args.expected_driver_sha256, '1' * 64)
        self.assertEqual(args.expected_compiler_sha256, '2' * 64)

    def test_lexer_dispatch_preserves_strict_gates(self):
        a = {'schema': 'stable_native_lexer/v1'}
        b = {'schema': 'stable_native_lexer/v1'}
        for changed in (False, True):
            compare = self.invoke(a, b, allow_compiler_change=changed,
                                  module_name='stable_resident_lexer_suite')
            if changed:
                compare.assert_called_once_with(a, b, 10, allow_compiler_change=True)
            else:
                compare.assert_called_once_with(a, b, 10)
        with self.assertRaisesRegex(SystemExit, 'only synthetic'):
            self.invoke(a, b, require_valid=True, module_name='stable_resident_lexer_suite')

    def test_root_lexer_cli_requires_hash_attestations_and_uses_fn(self):
        import stable_resident_lexer_suite as adapter
        argv = ['suite.py', 'stable-native-lexer', '--compiler', '/fixture/rustc',
                '--output', '/fixture/output', '--items', '8', '-n', '1',
                '--expected-driver-sha256', '1' * 64,
                '--expected-compiler-sha256', '2' * 64]
        with patch.object(adapter, 'run') as run, patch.object(sys, 'argv', argv):
            suite.main()
        run.assert_called_once()
        args = run.call_args.args[0]
        self.assertEqual(args.items, 8)
        self.assertEqual(args.expected_driver_sha256, '1' * 64)
        self.assertEqual(args.expected_compiler_sha256, '2' * 64)

    def test_automatic_record_upload_is_unique_opt_out_and_fail_open(self):
        import stable_resident_leaf_suite as adapter
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / 'unique-leaf-run'
            output.mkdir()
            receipt = output / 'receipt.json'
            receipt.write_text('{}')
            with patch.dict(adapter.os.environ, {'JUSTRUST_RECORDS': 'off'}), \
                    patch.object(adapter.subprocess, 'run') as upload:
                adapter.upload_result(output)
                upload.assert_not_called()
            result = types.SimpleNamespace(stderr='queued', stdout='')
            with patch.dict(adapter.os.environ, {'JUSTRUST_RECORDS': 'on'}), \
                    patch.object(adapter.subprocess, 'run', return_value=result) as upload, \
                    patch.object(sys, 'stderr', io.StringIO()):
                adapter.upload_result(output)
                self.assertEqual(upload.call_args.args[0][1:],
                                 ['records', 'put', 'bench', 'unique-leaf-run', str(receipt)])
            with patch.dict(adapter.os.environ, {'JUSTRUST_RECORDS': 'on'}), \
                    patch.object(adapter.subprocess, 'run', side_effect=OSError('offline')), \
                    patch.object(sys, 'stderr', io.StringIO()):
                adapter.upload_result(output)
            self.assertEqual(receipt.read_text(), '{}')

    def test_synthetic_validity_gate_is_not_silently_bypassed(self):
        with self.assertRaisesRegex(SystemExit, 'only synthetic'):
            self.invoke({'schema': 'stable_native_leaf/v1'},
                        {'schema': 'stable_native_leaf/v1'}, require_valid=True)


if __name__ == '__main__':
    unittest.main()
