#!/usr/bin/env python3
"""Mock-only contract tests. No native builds or executions."""
import copy
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import stable_resident_lexer as lexer


def response(hits=0, misses=0, rejected=0, entries=0, size=0, status=0):
    return dict(executed=True, status=status, owned={}, lexer=dict(
        hits=hits, misses=misses, rejected=rejected, retained_entries=entries,
        retained_estimated_bytes=size))


class Contract(unittest.TestCase):
    def test_counters_missing(self):
        for key in lexer.COUNTERS:
            item = response()
            del item['lexer'][key]
            with self.subTest(key=key), self.assertRaises(AssertionError):
                lexer.validate_lexer(item, None, 'control', 'cold')
        with self.assertRaises(AssertionError):
            lexer.validate_lexer({}, None, '1', 'cold')

    def test_disabled_nonzero(self):
        for key in lexer.COUNTERS:
            item = response()
            item['lexer'][key] = 1
            with self.subTest(key=key), self.assertRaises(AssertionError):
                lexer.validate_lexer(item, None, 'control', 'cold')
        lexer.validate_lexer(response(), None, 'control', 'cold')

    def test_hit_failure_does_not_pass(self):
        with self.assertRaises(AssertionError):
            lexer.validate_lexer(response(2, 2, 1, 2, 100, status=1),
                                 response(0, 2, 1, 2, 100)['lexer'], '1', 'same')
        # Lexer-only reuse of an unchanged sibling is valid during an error.
        lexer.validate_lexer(response(2, 2, 2, 2, 100, status=1),
                             response(0, 2, 1, 2, 100)['lexer'], '1', 'error')

    def test_unchanged_cross_require_hits_and_misses(self):
        prior = response(1, 2, 1, 2, 100)['lexer']
        lexer.validate_lexer(response(3, 2, 2, 2, 100), prior, '1', 'same')
        lexer.validate_lexer(response(2, 3, 2, 3, 150), prior, '1', 'cross')
        for item in (response(1, 3, 2, 3, 150), response(2, 2, 2, 3, 150),
                     response(3, 3, 2, 3, 150)):
            with self.assertRaises(AssertionError):
                lexer.validate_lexer(item, prior, '1', 'cross')

    def test_bounds_and_counter_types(self):
        for item in (response(entries=129), response(size=8 * 1024 * 1024 + 1),
                     response(hits=True), response(misses=-1)):
            with self.assertRaises(AssertionError):
                lexer.validate_lexer(item, None, '1', 'cold')
        lexer.validate_lexer(response(entries=128, size=8 * 1024 * 1024), None, '1', 'cold')

    def test_fallback_and_error_require_rejection(self):
        for kind in ('fallback', 'error'):
            with self.assertRaises(AssertionError):
                lexer.validate_lexer(response(), None, '1', kind)
        with self.assertRaises(AssertionError):
            lexer.validate_lexer(response(rejected=1), None, '1', 'error')

    def test_control_native_gate_is_zero(self):
        with patch.dict('os.environ', {'JUSTRUST_RUSTC_RESIDENT_LEXER': '1',
                                     'JUSTRUST_RUSTC_RESIDENT_TYPED': 'leaf'}):
            env = lexer.environment('control')
        self.assertEqual(env['JUSTRUST_RUSTC_RESIDENT_LEXER'], '0')
        self.assertNotIn('JUSTRUST_RUSTC_RESIDENT_TYPED', env)
        self.assertEqual(lexer.environment('1')['JUSTRUST_RUSTC_RESIDENT_LEXER'], '1')

    def test_environment_attribution(self):
        with patch.dict('os.environ', {'JUSTRUST_RUSTC_RESIDENT_TYPED': 'leaf',
                                     'JUSTRUST_RUSTC_RESIDENT_OTHER': '1'}):
            env = lexer.environment('1')
        self.assertEqual({k: v for k, v in env.items() if k.startswith('JUSTRUST_RUSTC_RESIDENT_')},
                         {'JUSTRUST_RUSTC_RESIDENT_LEXER': '1'})

    def test_unique_cross_edit_sources(self):
        previous, seen = None, set()
        for _, current, _, kind in lexer.cases():
            lexer.validate_transition(previous, current, kind, seen)
            seen.update(current.values())
            previous = current
        a, b = lexer.sources(), lexer.sources(b=17)
        with self.assertRaises(AssertionError):
            lexer.validate_transition(a, b, 'cross', set(b.values()))
        with self.assertRaises(AssertionError):
            lexer.validate_transition(a, lexer.sources(a=23, b=17), 'cross', set())

    def test_default_cases_preserve_frozen_contract(self):
        self.assertEqual(lexer.cases(), lexer.cases(common_syntax=False))
        self.assertEqual([(row[0], row[3]) for row in lexer.cases()], [
            ('initial', 'cold'), ('unchanged', 'same'), ('unrelated-edit', 'cross'),
            ('relevant-edit', 'cross'), ('delimiter-error', 'error'),
            ('recovery', 'recovery'), ('unsupported-comment', 'fallback'),
            ('unsupported-raw-ident', 'fallback'), ('unsupported-unicode', 'fallback'),
            ('unsupported-string', 'fallback')])

    def test_common_case_transitions_and_repeat_oracles(self):
        previous, seen = None, set()
        rows = lexer.cases(common_syntax=True)
        self.assertEqual(len(rows), 23)
        repeats = []
        for index, (name, current, expected, kind) in enumerate(rows):
            lexer.validate_transition(previous, current, kind, seen)
            if kind == 'same' and name.startswith('common-'):
                self.assertEqual(rows[index - 1][1:3], (current, expected))
                repeats.append(name)
            if name.startswith('common-'):
                self.assertIn(b'extern "C"', current['fixture.rs'])
                self.assertEqual(current['module_b.rs'], lexer.sources(a=31, b=19)['module_b.rs'])
            seen.update(current.values())
            previous = current
        self.assertEqual(len(repeats), 5)
        by_name = {row[0]: row for row in rows}
        self.assertEqual(by_name['common-comment-relevant-edit'][2:], ((69, 22), 'cross'))
        self.assertIn(b'#[inline]', by_name['common-attribute'][1]['module_a.rs'])
        self.assertIn(b'/* outer /* nested */ terminated */', by_name['common-nested-block'][1]['module_a.rs'])
        for name in ('raw-ident', 'unicode', 'string', 'line-doc', 'block-doc'):
            self.assertEqual(by_name['unsupported-' + name][3], 'common-fallback')
        self.assertEqual(by_name['unsupported-unterminated-comment'][2:], (None, 'error'))

    def test_common_warmup_and_fallback_counters(self):
        prior = response(5, 8, 4, 4, 100)['lexer']
        lexer.validate_lexer(response(5, 10, 5, 5, 150), prior, '1', 'common-warmup')
        lexer.validate_lexer(response(6, 10, 5, 5, 150), prior, '1', 'common-warmup')
        with self.assertRaises(AssertionError):
            lexer.validate_lexer(response(6, 10, 5, 5, 150, status=1), prior, '1', 'common-warmup')
        lexer.validate_lexer(response(6, 10, 6, 5, 150), prior, '1', 'common-fallback')
        for item in (response(6, 10, 5, 5, 150), response(7, 10, 6, 5, 150),
                     response(6, 10, 6, 5, 150, status=1)):
            with self.assertRaises(AssertionError):
                lexer.validate_lexer(item, prior, '1', 'common-fallback')
        lexer.validate_lexer(response(), None, 'control', 'common-warmup')
        lexer.validate_lexer(response(), None, 'control', 'common-fallback')
        with self.assertRaises(AssertionError):
            lexer.validate_lexer(response(hits=1), None, 'control', 'common-warmup')

    def test_common_cli_is_opt_in(self):
        for flag, expected in (([], False), (['--common-syntax'], True)):
            with patch('sys.argv', ['lexer', '--compiler', '/frozen', '--out', '/evidence', *flag]), \
                 patch.object(lexer, 'capture') as capture, patch('builtins.print'):
                lexer.main()
            capture.assert_called_once_with(Path('/frozen'), Path('/evidence'), 180,
                                            common_syntax=expected)

    def test_object_only(self):
        args = lexer.arguments(Path('/fixture'))
        self.assertEqual([v for v in args if v.startswith('--emit=')], ['--emit=obj=/fixture/fixture.o'])
        self.assertFalse(any('incremental' in v or 'llvm-ir' in v or 'mir=' in v for v in args))

    def test_exact_diagnostic_and_runtime_mismatch(self):
        def record(nonce):
            return dict(compile=dict(status=0, stdout_b64='', stderr_b64='', pid=42),
                        runtime=dict(nonce=nonce, execution=dict(status=0, stderr_b64='',
                            stdout_b64=lexer.shared.encoded((nonce + ':9:14\n').encode()))))
        pair = dict(fresh=record('fresh'), owned=record('owned'))
        lexer.owned.validate_pair(pair, '9:14', 42)
        for target, key, value in [('compile', 'stderr_b64', 'wrong'),
                                   ('compile', 'pid', 43)]:
            bad = copy.deepcopy(pair)
            bad['owned'][target][key] = value
            with self.assertRaises(AssertionError):
                lexer.owned.validate_pair(bad, '9:14', 42)
        bad = copy.deepcopy(pair)
        bad['owned']['runtime']['execution']['stdout_b64'] = pair['fresh']['runtime']['execution']['stdout_b64']
        with self.assertRaises(AssertionError):
            lexer.owned.validate_pair(bad, '9:14', 42)

    def test_mock_capture_failure_retains_observation(self):
        class FakeResident:
            def __init__(self, compiler, root, mode, timeout):
                self.process = type('Process', (), {'pid': 42})()
                self.root = root
            def compile(self, argv):
                return dict(status=1, pid=42, stdout_b64='', stderr_b64='owned-error')
            def close(self):
                pass
        with tempfile.TemporaryDirectory() as temp:
            out = Path(temp) / 'evidence'
            with patch.object(lexer, 'LexerResident', FakeResident), \
                 patch.object(lexer.shared, 'identity', return_value={'digest': 'frozen'}), \
                 patch.object(lexer.shared, 'run', return_value=dict(
                     status=1, stdout_b64='', stderr_b64='fresh-error')):
                with self.assertRaisesRegex(AssertionError, 'diagnostics differ'):
                    lexer.capture('/frozen/compiler', out)
            receipt = json.loads((out / 'receipt.json').read_text())
            self.assertEqual(receipt['verdict'], 'failed')
            row = receipt['modes'][0]['observations'][0]
            self.assertEqual(row['modes']['owned']['compile']['stderr_b64'], 'owned-error')
            self.assertEqual(row['modes']['fresh']['compile']['stderr_b64'], 'fresh-error')
            self.assertNotIn('runtime', row['modes']['owned'])

    def test_failure_receipt(self):
        with tempfile.TemporaryDirectory() as temp:
            out = Path(temp) / 'evidence'
            with patch.object(lexer.shared, 'identity', side_effect=RuntimeError('frozen missing')):
                with self.assertRaisesRegex(RuntimeError, 'frozen missing'):
                    lexer.capture('/not/a/compiler', out)
            receipt = json.loads((out / 'receipt.json').read_text())
            self.assertEqual(receipt['verdict'], 'failed')
            self.assertIn('frozen missing', receipt['error'])


if __name__ == '__main__':
    unittest.main()
