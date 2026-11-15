#!/usr/bin/env python3
"""Synthetic pinned-runner validation, not compiler correctness evidence."""
import argparse
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import hybrid_workload_pinned as pinned


NAMES = pinned.resident.NAMES


def transcript(nonce=1, failed=False):
    return '\n'.join('test ' + name + ' ... ' + ('FAILED' if failed and name == pinned.resident.EDIT_TEST else 'ok') for name in NAMES) + '\n' + f'JR_RUNTIME_EXECUTED_{nonce}\n' + ('JR_RUNTIME_FAILURE\n' if failed else '') + f'test result: {"FAILED" if failed else "ok"}. {3 if failed else 4} passed; {1 if failed else 0} failed; 0 ignored; 0 measured; 100 filtered out\n'


FAKE = '''#!/usr/bin/env python3
import os, pathlib, re, sys
text = pathlib.Path('src/summary.rs').read_text()
if 'prewarm' in sys.argv:
    sys.exit(0)
if 'JR_RUNTIME_TYPE_ERROR' in text:
    print('error[E0308]: JR_RUNTIME_TYPE_ERROR')
    sys.exit(101)
nonce = re.search(r'JR_RUNTIME_EXECUTED_(\\d+)', text)[1]
with open(os.environ['HYBRID_SIDE_EFFECT'], 'a') as f: f.write('nonce=' + nonce + '\\n')
names = NAMES_VALUE
failed = 'JR_RUNTIME_FAILURE' in text
for name in names:
    print('test ' + name + ' ... ' + ('FAILED' if failed and name.endswith('shares_overlapping_time') else 'ok'))
print('JR_RUNTIME_EXECUTED_' + nonce)
if failed: print('JR_RUNTIME_FAILURE')
print('test result: ' + ('FAILED. 3 passed; 1 failed;' if failed else 'ok. 4 passed; 0 failed;') + ' 0 ignored; 0 measured; 100 filtered out')
if not failed and os.environ.get('JUSTRUST_HYBRID_PREWARM') == 'after-success':
    with (pathlib.Path(os.environ['JR_ENGINE_HOME']) / 'prewarm.log').open('a') as f:
        f.write('AUTOMATIC_SUCCESS\\n')
    print('justrust hybrid: queued background preparation')
sys.exit(101 if failed else 0)
'''.replace('NAMES_VALUE', repr(NAMES))


class PinnedContracts(unittest.TestCase):
    def test_nonce_is_runtime_observed(self):
        result, _ = pinned.accepted({'status': 0, 'timed_out': False, 'error': None}, transcript(), ['nonce=1'], 1, 'edit')
        self.assertTrue(result)
        result, _ = pinned.accepted({'status': 0, 'timed_out': False, 'error': None}, transcript(), ['nonce=1'], 2, 'edit')
        self.assertFalse(result)

    def test_assert_error_checks_all_tests(self):
        result, _ = pinned.accepted({'status': 101, 'timed_out': False, 'error': None}, transcript(failed=True), ['nonce=1'], 1, 'test-failure')
        self.assertTrue(result)
        result, _ = pinned.accepted({'status': 101, 'timed_out': False, 'error': None}, transcript(failed=True), ['nonce=1', 'nonce=1'], 1, 'test-failure')
        self.assertFalse(result)

    def test_anchor_must_be_unique(self):
        with self.assertRaises(ValueError):
            pinned.edited(b'no anchor', 1)

    def test_type_error_has_no_side_effect_code(self):
        content = pinned.edited(b'fn shares_overlapping_time() {}', 1, 'type-error')
        self.assertIn(b'JR_RUNTIME_TYPE_ERROR', content)
        self.assertNotIn(b'HYBRID_SIDE_EFFECT', content)

    def run_matrix(self, automatic=False):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            source = root / 'fixture'
            (source / 'src').mkdir(parents=True)
            original = b'fn shares_overlapping_time() {}'
            (source / 'src/summary.rs').write_bytes(original)
            fake = root / 'fake'
            fake.write_text(FAKE)
            fake.chmod(0o700)
            args = argparse.Namespace(output=root / 'out', source=source, justrust=fake,
                                      engine=fake, toolchain=None, env=[], samples=2, timeout=5,
                                      prewarm=not automatic, cold_route_regex=None, warm_route_regex=None,
                                      automatic=automatic, automatic_ready_regex='AUTOMATIC_SUCCESS' if automatic else None)
            # Explicitly synthetic: bypass pin only here, never in the executable.
            with patch.object(pinned.resident, 'pinned_source'):
                self.assertEqual(pinned.execute(args), 0)
            report = json.loads((args.output / 'report.json').read_text())
            self.assertEqual(report['status'], 'passed')
            self.assertEqual(len(report['rows']), 16 if automatic else 17)
            self.assertEqual((source / 'src/summary.rs').read_bytes(), original)
            self.assertFalse(report['routing_evidence_complete'])
            self.assertTrue(all(Path(row['raw_prefix'] + '.stdout').is_file() for row in report['rows']))

    def test_synthetic_end_to_end(self):
        self.run_matrix()

    def test_synthetic_automatic_prewarm(self):
        self.run_matrix(automatic=True)


if __name__ == '__main__':
    unittest.main()
