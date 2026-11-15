import contextlib
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import resident_tier_full_gate as gate

TEXT = ('running 1 test\ntest example::works ... ok\n\n'
        'test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n')


class FullGateTests(unittest.TestCase):
    def run_gate(self, native_text=TEXT, stop_status=0):
        root = Path(self.temp.name)
        engine, client, source, tc, output = [root / p for p in ('engine', 'client', 'source', 'tc', 'result')]
        engine.write_bytes(b'engine')
        client.write_bytes(b'client')
        source.mkdir()
        tc.mkdir()
        called = []
        def process(argv, cwd, env, prefix, timeout):
            called.append(list(argv))
            return dict(status=stop_status if argv[-1] == 'stop' else 0,
                        timed_out=False, error=None, elapsed_seconds=1.0)
        def captured(prefix):
            text = {'current': 'justrust log 20261115-065547143-557208',
                    'current-recorded': TEXT, 'resident': native_text, 'stop': ''}[prefix.name]
            return {}, text, None
        argv = ['gate', '--engine', str(engine), '--justrust', str(client), '--source', str(source),
                '--toolchain', str(tc), '--output', str(output)]
        with patch('sys.argv', argv), patch.object(gate.tier.shared, 'pinned_source'), \
             patch.object(gate.tier.shared, 'source_hashes', return_value={'input': 'hash'}), \
             patch.object(gate.tier.shared, 'environment', return_value={}), \
             patch.object(gate.tier.runtime, 'run_process', side_effect=process), \
             patch.object(gate.tier.matrix, 'captured', side_effect=captured), \
             contextlib.redirect_stdout(io.StringIO()):
            try:
                status = gate.main()
            except ValueError:
                status = 'raised'
        return status, json.loads((output / 'result.json').read_text()), called

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)

    def test_exact_test_contract_and_cleanup_pass(self):
        status, report, calls = self.run_gate()
        self.assertEqual(status, 0)
        self.assertEqual(report['status'], 'passed')
        self.assertTrue(report['cleanup_succeeded'])
        self.assertTrue(report['tools_unchanged'])
        self.assertTrue(report['source_unchanged'])
        self.assertEqual(calls[-1][-1], 'stop')

    def test_missing_tests_fail_and_still_stop(self):
        status, report, calls = self.run_gate(native_text='no tests')
        self.assertEqual(status, 'raised')
        self.assertEqual(report['status'], 'failed')
        self.assertEqual(calls[-1][-1], 'stop')

    def test_cleanup_failure_cannot_pass(self):
        status, report, _ = self.run_gate(stop_status=1)
        self.assertEqual(status, 1)
        self.assertFalse(report['cleanup_succeeded'])
        self.assertEqual(report['status'], 'failed')


if __name__ == '__main__':
    unittest.main()
