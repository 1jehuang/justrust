"""Pure Python mock tests. Never starts rustc, a native worker or a build."""
import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import stable_resident_ast as ast


def response(stats=None):
    return dict(executed=True, status=0, pid=731,
                owned=dict(revisions=1, contexts=1, retirement_reason=None),
                ast=dict.fromkeys(ast.COUNTERS, 0) if stats is None else stats,
                live=dict.fromkeys(ast.LIVE_COUNTERS, 0),
                lexer=dict.fromkeys(ast.COUNTERS, 0), stdout_b64='', stderr_b64='')


def observable(status=0, stdout=b'', stderr=b''):
    return dict(status=status, stdout_b64=ast.shared.encoded(stdout),
                stderr_b64=ast.shared.encoded(stderr))


def mock_compile(root):
    source = (root / 'fixture.rs').read_text()
    if '& 255 ) }' in source or '->bool' in source:
        return observable(1, stderr=b'original diagnostic: exact bytes\n')
    (root / 'libast_fixture.rlib').write_bytes(b'not a real rlib:' + source.encode())
    return observable()


class FakeResident:
    drop_hit = False
    corrupt_diagnostic = False
    instances = []

    def __init__(self, compiler, root, mode, timeout):
        self.root, self.mode = root, mode
        self.command = [str(compiler), '--jr-resident-owned']
        self.process = SimpleNamespace(pid=731)
        self.stats = dict.fromkeys(ast.COUNTERS, 0)
        self.cache = set()
        self.args = None
        self.closed = False
        self.requests = 0
        self.instances.append(self)
        for filename in ('resident.stdout', 'resident.stderr', 'resident.jsonl'):
            (root / filename).write_bytes(b'original full stream\n')

    def compile(self, argv):
        if self.args is not None:
            assert self.args == argv
        self.args = list(argv)
        self.requests += 1
        source = (self.root / 'fixture.rs').read_text()
        result = response()
        result.update(mock_compile(self.root))
        result['owned']['revisions'] = self.requests
        if self.mode == '1':
            if any(token in source for token in ('macro_rules!', '#', '& 255 ) }')):
                self.stats['rejected'] += 1
            elif source in self.cache:
                if not self.drop_hit:
                    self.stats['hits'] += 1
            else:
                self.stats['misses'] += 1
                self.cache.add(source)
                self.stats['retained_entries'] = len(self.cache)
                self.stats['retained_estimated_bytes'] = sum(len(s) for s in self.cache)
        result['ast'] = dict(self.stats)
        if self.corrupt_diagnostic and result['status'] == 1:
            result['stderr_b64'] = ast.shared.encoded(b'wrong diagnostic\n')
        return result

    def close(self):
        self.closed = True


def mock_run(command, timeout, cwd, env):
    root = Path(cwd)
    if '--crate-type=rlib' in command:
        return mock_compile(root)
    if str(root / 'consumer.rs') in command:
        (root / 'consumer').write_bytes(b'not an executable')
        return observable()
    if str(command[0]) == str(root / 'consumer'):
        nonce = command[1]
        source = (root / 'fixture.rs').read_text()
        helper = (root / 'helper.rs').read_text()
        a = 19 if '^ 19' in source else 7
        b = 23 if '^ 23' in helper else 11
        data = ''.join(f'{nonce}:{x}:{(x ^ a) & 255}:{(x ^ b) & 255}\n' for x in ast.EDGES)
        return observable(stdout=data.encode())
    raise AssertionError('Unexpected native command in mock: ' + repr(command))


class StatsTests(unittest.TestCase):
    def test_explicit_isolated_environment(self):
        with patch.dict('os.environ', {'JUSTRUST_RUSTC_RESIDENT_AST': '1',
                                      'JUSTRUST_RUSTC_RESIDENT_OTHER': '1'}):
            env = ast.environment('0')
        self.assertEqual(env['JUSTRUST_RUSTC_RESIDENT_AST'], '0')
        self.assertEqual(env['JUSTRUST_RUSTC_RESIDENT_TYPED'], '0')
        self.assertEqual(env['JUSTRUST_RUSTC_RESIDENT_LEXER'], '0')
        self.assertNotIn('JUSTRUST_RUSTC_RESIDENT_OTHER', env)
        with self.assertRaises(ValueError):
            ast.environment('control')

    def test_disabled_all_zero(self):
        ast.validate_stats(response(), None, '0', 'cold')
        for key in ast.COUNTERS:
            result = response()
            result['ast'][key] = 1
            with self.subTest(key=key), self.assertRaises(AssertionError):
                ast.validate_stats(result, None, '0', 'cold')

    def test_live_and_lexer_cannot_be_substituted(self):
        for field, keys in (('live', ast.LIVE_COUNTERS), ('lexer', ast.COUNTERS)):
            for key in keys:
                result = response()
                result[field][key] = 1
                with self.subTest(field=field, key=key), self.assertRaises(AssertionError):
                    ast.validate_stats(result, None, '0', 'cold')

    def test_missing_and_boolean_counters_rejected(self):
        for field in ('live', 'lexer', 'ast'):
            result = response()
            del result[field]
            with self.subTest(field=field), self.assertRaises(AssertionError):
                ast.validate_stats(result, None, '0', 'cold')
        result = response()
        result['ast']['hits'] = False
        with self.assertRaises(AssertionError):
            ast.validate_stats(result, None, '0', 'cold')

    def test_cold_miss_and_exact_hits(self):
        initial = dict(hits=0, misses=1, rejected=0, retained_entries=1, retained_estimated_bytes=80)
        ast.validate_stats(response(initial), None, '1', 'cold')
        warmed = dict(initial, hits=1)
        ast.validate_stats(response(warmed), initial, '1', 'hit')
        for hits in (0, 2):
            with self.subTest(hits=hits), self.assertRaises(AssertionError):
                ast.validate_stats(response(dict(initial, hits=hits)), initial, '1', 'hit')
        with self.assertRaises(AssertionError):
            ast.validate_stats(response(warmed), initial, '1', 'miss')

    def test_recovery_respects_exact_source_identity(self):
        previous = dict(hits=2, misses=2, rejected=1, retained_entries=2, retained_estimated_bytes=160)
        ast.validate_stats(response(dict(previous, hits=3)), previous, '1', 'recover')
        ast.validate_stats(response(dict(previous, misses=3)), previous, '1', 'recover')
        for update in ({}, dict(hits=3, misses=3), dict(hits=4), dict(misses=4), dict(misses=3, rejected=2)):
            with self.subTest(update=update), self.assertRaises(AssertionError):
                ast.validate_stats(response(dict(previous, **update)), previous, '1', 'recover')
        kinds = {row[0]: row[-1] for row in ast.cases()}
        self.assertEqual(kinds['unrelated-helper-edit'], 'hit')
        self.assertEqual(kinds['unchanged'], 'hit')
        self.assertEqual(kinds['recovery'], 'recover')
        self.assertEqual(kinds['final-recovery'], 'recover')

    def test_root_only_no_helper_misses(self):
        initial = dict(hits=0, misses=1, rejected=0, retained_entries=1, retained_estimated_bytes=80)
        with self.assertRaises(AssertionError):
            ast.validate_stats(response(dict(initial, misses=2)), None, '1', 'cold')
        with self.assertRaises(AssertionError):
            ast.validate_stats(response(dict(initial, hits=1, misses=2)), initial, '1', 'hit')

    def test_rejection_no_admission(self):
        previous = dict(hits=1, misses=1, rejected=0, retained_entries=1, retained_estimated_bytes=80)
        rejected = dict(previous, rejected=1)
        ast.validate_stats(response(rejected), previous, '1', 'reject')
        for key in ast.COUNTERS[3:]:
            with self.subTest(key=key), self.assertRaises(AssertionError):
                ast.validate_stats(response(dict(rejected, **{key: previous[key] + 1})), previous, '1', 'reject')

    def test_parse_success_type_error_can_hit(self):
        previous = dict(hits=0, misses=1, rejected=0, retained_entries=1, retained_estimated_bytes=80)
        result = response(dict(previous, hits=1))
        result['status'] = 1
        ast.validate_stats(result, previous, '1', 'hit')

    def test_cumulative_not_gauges(self):
        previous = dict(hits=2, misses=2, rejected=2, retained_entries=2, retained_estimated_bytes=160)
        result = response(dict(previous, hits=1))
        with self.assertRaises(AssertionError):
            ast.validate_stats(result, previous, '1', 'hit')

    def test_fixed_ordinary_rlib_and_consumer(self):
        argv = ast.arguments(Path('/fixed'))
        self.assertIn('--crate-type=rlib', argv)
        self.assertIn('--emit=link', argv)
        self.assertFalse(any('incremental' in x or 'mir' in x or 'llvm' in x for x in argv))
        self.assertIn('ast_fixture::helper::value(x)', ast.consumer_source())
        self.assertIn('4294967295u32', ast.consumer_source())
        cases = ast.cases()
        self.assertEqual(cases[0][1], cases[2][1])
        self.assertNotEqual(cases[0][2], cases[2][2])
        self.assertEqual(cases[4][1], cases[6][1])
        self.assertNotIn('#', cases[0][1])


class CaptureTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.out = Path(self.temp.name) / 'evidence'
        self.identity = dict(sha256='launcher', drivers=[dict(sha256='driver')], source_id='frozen')
        self.patches = [patch.object(ast, 'AstResident', FakeResident),
                        patch.object(ast.shared, 'run', side_effect=mock_run),
                        patch.object(ast.shared, 'identity', return_value=self.identity)]
        for mocked in self.patches:
            mocked.start()
            self.addCleanup(mocked.stop)
        FakeResident.instances = []
        FakeResident.drop_hit = FakeResident.corrupt_diagnostic = False

    def test_capture_complete_mock_only(self):
        report = ast.capture('/frozen/rustc', self.out)
        self.assertEqual(report['verdict'], 'passed')
        self.assertEqual(report['identity_before'], self.identity)
        self.assertEqual(report['identity_after'], self.identity)
        self.assertEqual([m['mode'] for m in report['modes']], ['0', '1'])
        self.assertTrue(all(w.closed for w in FakeResident.instances))
        for mode in report['modes']:
            self.assertEqual(len(mode['observations']), len(ast.cases()))
            self.assertEqual(mode['pid'], 731)
            root = Path(mode['cwd'])
            self.assertEqual((root / 'resident.stderr').read_bytes(), b'original full stream\n')
            self.assertTrue((root / '00-fixture.rs').exists())
            row = mode['observations'][0]
            self.assertEqual(row['records']['resident']['compile']['ast']['hits'], 0)
            self.assertTrue(Path(row['records']['fresh']['artifact']['path']).exists())
            nonces = [r['runtime']['nonce'] for r in row['records'].values()]
            self.assertEqual(len(set(nonces)), 2)
        self.assertEqual(json.loads((self.out / 'receipt.json').read_text())['verdict'], 'passed')

    def test_failed_hit_receipt_keeps_originals_and_continues(self):
        FakeResident.drop_hit = True
        report = ast.capture('/frozen/rustc', self.out)
        self.assertEqual(report['verdict'], 'failed')
        mode = report['modes'][1]
        self.assertEqual(len(mode['observations']), len(ast.cases()))
        self.assertEqual(mode['observations'][1]['records']['resident']['compile']['ast']['hits'], 0)
        self.assertIn('Exact root AST hit', mode['observations'][1]['acceptance_error'])
        self.assertEqual(mode['observations'][-1]['name'], 'final-recovery')
        self.assertEqual((self.out / 'ast' / 'resident.jsonl').read_bytes(), b'original full stream\n')

    def test_exact_diagnostics_required(self):
        FakeResident.corrupt_diagnostic = True
        report = ast.capture('/frozen/rustc', self.out)
        self.assertEqual(report['verdict'], 'failed')
        row = report['modes'][1]['observations'][5]
        self.assertIn('Exact status/stdout/diagnostics', row['acceptance_error'])
        self.assertNotEqual(row['records']['fresh']['compile']['stderr_b64'],
                            row['records']['resident']['compile']['stderr_b64'])

    def test_duplicate_runtime_nonce_fails(self):
        with patch.object(ast.uuid, 'uuid4', return_value=SimpleNamespace(hex='reused')):
            report = ast.capture('/frozen/rustc', self.out)
        self.assertEqual(report['verdict'], 'failed')
        self.assertIn('new nonce', report['modes'][0]['observations'][0]['acceptance_error'])

    def test_existing_output_never_overwritten(self):
        self.out.mkdir()
        (self.out / 'receipt.json').write_text('original')
        with self.assertRaises(FileExistsError):
            ast.capture('/frozen/rustc', self.out)
        self.assertEqual((self.out / 'receipt.json').read_text(), 'original')

    def test_identity_change_fails(self):
        with patch.object(ast.shared, 'identity', side_effect=[self.identity, {'sha256': 'changed'}]):
            report = ast.capture('/frozen/rustc', self.out)
        self.assertEqual(report['verdict'], 'failed')
        self.assertIn('changed', report['identity_error'])

    def test_exception_keeps_failed_receipts(self):
        with patch.object(ast.shared, 'run', side_effect=RuntimeError('mock transport failed')):
            with self.assertRaisesRegex(RuntimeError, 'mock transport failed'):
                ast.capture('/frozen/rustc', self.out)
        for receipt in (self.out / 'receipt.json', self.out / 'disabled' / 'receipt.json'):
            report = json.loads(receipt.read_text())
            self.assertEqual(report['verdict'], 'failed')
        self.assertTrue(FakeResident.instances[0].closed)

    def test_bad_timeout_does_not_create_output(self):
        for timeout in (0, -1, float('inf'), float('nan')):
            with self.subTest(timeout=timeout), self.assertRaises(ValueError):
                ast.capture('/frozen/rustc', self.out, timeout)
        self.assertFalse(self.out.exists())


if __name__ == '__main__':
    unittest.main()
