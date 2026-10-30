import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location('macro_input', Path(__file__).with_name('reuse-macro-input-accept.py'))
M = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(M)


class MacroInputTests(unittest.TestCase):
    def test_cached_families_have_real_entrypoints(self):
        for family, marker in [('derive', '#[proc_macro_derive(Input)]'), ('attribute', '#[proc_macro_attribute]')]:
            source = M.macro_source('env', family=family)
            self.assertIn(marker, source)
            self.assertIn('fn macro_value()', source)
            self.assertNotIn('fixture::input!()', M.consumer(0, family))
            self.assertNotEqual(M.consumer(0, family), M.consumer(1, family))

    def test_missing_hit_is_incomplete(self):
        with tempfile.TemporaryDirectory() as directory:
            h = M.Harness('/nonexistent/rustc', Path(directory))
            with patch.object(h, 'build_macro', return_value={'status': 0}), \
                 patch.object(h, 'compile_consumer', side_effect=lambda *args: {
                     'compile': {'status': 0, 'stdout': '', 'stderr': ''},
                     'runtime': {'status': 0, 'stdout': M.expected_output('constant', next(
                         s for s in M.states('constant') if M.consumer(s['body'], 'derive') ==
                         (h.root / 'consumer.rs').read_text())), 'stderr': ''}}), \
                 patch.object(h, 'cache_evidence', return_value={'hit': False}):
                case = h.scenario('constant', 'derive')
            self.assertEqual(case['status'], 'incomplete')
            self.assertEqual(case['coverage'], 'cached-family')

    def test_disk_load_evidence_is_query_specific(self):
        row = {'label': 'derive_macro_expansion', 'number_of_cache_hits': 5,
               'incremental_load_time': {'secs': 0, 'nanos': 0}}
        self.assertFalse(M.loaded_query({'query_data': [row]}, 'derive_macro_expansion'))
        row['incremental_load_time']['nanos'] = 1
        self.assertTrue(M.loaded_query({'query_data': [row]}, 'derive_macro_expansion'))
        self.assertFalse(M.loaded_query({'query_data': [row]}, 'attr_macro_expansion'))

    def test_actual_body_edits_and_revision(self):
        for kind in ('constant', 'env', 'file', 'tracked_env', 'tracked_path'):
            states = M.states(kind)
            self.assertEqual(len({M.consumer(s['body']) for s in states}), len(states))
            self.assertEqual(states[-1]['revision'], 1)
            self.assertIn('fixture::input!()', M.consumer(0))

    def test_source_inputs_and_apis(self):
        self.assertIn('std::env::var("JR_MACRO_INPUT")', M.macro_source('env'))
        self.assertIn('std::fs::read_to_string(path)', M.macro_source('file'))
        self.assertIn('proc_macro::tracked::env_var', M.macro_source('tracked_env'))
        self.assertIn('proc_macro::tracked_path::path', M.macro_source('tracked_path', api='legacy'))
        self.assertIn('{}:1', M.macro_source('constant', 1))

    def test_exact_comparison_rejects_every_observable_difference(self):
        base = {'status': 0, 'stdout': 'ok', 'stderr': ''}
        for key, value in [('status', 1), ('stdout', 'stale'), ('stderr', 'warning')]:
            self.assertFalse(M.equivalent(base, dict(base, **{key: value})))
        self.assertEqual(M.normalize({'stderr': '/a/b/input.rs'}, Path('/a/b')), {'stderr': '<WORK>/input.rs'})

    def test_unsupported_api_preserves_both_probes(self):
        with tempfile.TemporaryDirectory() as directory:
            h = M.Harness('/nonexistent/rustc', Path(directory))
            with patch.object(M, 'execute', return_value={'status': 1, 'stdout': '', 'stderr': 'unsupported API'}):
                case = h.scenario('tracked_env')
            self.assertEqual(case['status'], 'unsupported')
            self.assertEqual([o['api'] for o in h.observations], ['modern', 'legacy'])
            self.assertTrue(all(o['result']['stderr'] == 'unsupported API' for o in h.observations))

    def test_mocked_matrix_and_stale_cache_detection(self):
        for stale in (False, True):
            with self.subTest(stale=stale), tempfile.TemporaryDirectory() as directory:
                h = M.Harness('/nonexistent/rustc', Path(directory))
                calls = []
                def compile_consumer(incremental, enabled):
                    calls.append((str(incremental), enabled))
                    body = int((h.root / 'consumer.rs').read_text().split(', ')[-1].split(')')[0])
                    revision = 1 if body == 3 else 0
                    value = h.env['JR_MACRO_INPUT']
                    if stale and enabled and body == 2 and 'fresh' not in str(incremental):
                        value = 'alpha'
                    return {'compile': {'status': 0, 'stdout': '', 'stderr': ''},
                            'runtime': {'status': 0, 'stdout': f'{value}:{revision}|{body}\n', 'stderr': ''}}
                with patch.object(h, 'build_macro', return_value={'status': 0}), patch.object(h, 'compile_consumer', side_effect=compile_consumer):
                    case = h.scenario('env')
                self.assertEqual(case['status'], 'fail' if stale else 'pass')
                self.assertEqual(len(calls), 24)
                self.assertEqual(calls[4][0], calls[5][0])
                self.assertEqual(calls[0][0], calls[6][0])
                self.assertNotEqual(calls[1][0], calls[7][0])
                if stale:
                    self.assertEqual(case['steps'][2]['records']['on']['runtime']['stdout'], 'alpha:0|2\n')

    def test_failed_compile_never_executes_old_binary(self):
        with tempfile.TemporaryDirectory() as directory:
            h = M.Harness('/nonexistent/rustc', Path(directory))
            binary = h.root / 'consumer'
            binary.write_text('old executable')
            with patch.object(M, 'execute', return_value={'status': 1, 'stdout': '', 'stderr': 'compile error'}) as execute:
                record = h.compile_consumer(h.root / 'incremental', True)
            self.assertIsNone(record['runtime'])
            self.assertFalse(binary.exists())
            self.assertEqual(execute.call_count, 1)
            self.assertIn('-Zcache-proc-macros=yes', execute.call_args.args[0])


if __name__ == '__main__':
    unittest.main()
