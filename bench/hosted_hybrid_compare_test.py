import subprocess
from pathlib import Path
import shlex
import tempfile
from types import SimpleNamespace
import unittest

import hosted_hybrid_compare as subject


class HostedHarnessTests(unittest.TestCase):
    def receipt(self, text, **changes):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'log'
            path.write_text(text)
            row = {'returncode': 0, 'timeout': False, 'error': None, **changes}
            return subject.parse_side_effects(row, path)

    def test_transport_messages_are_not_side_effects(self):
        self.assertEqual(self.receipt('hosted: ready\n["nonce=1"]\n'), ['nonce=1'])

    def test_empty_authoritative_list_is_valid(self):
        self.assertEqual(self.receipt('[]\n'), [])

    def test_failed_transport_never_means_empty_effects(self):
        for changed in ({'returncode': 1}, {'timeout': True}, {'error': 'failed'}):
            with self.subTest(changed=changed), self.assertRaises(ValueError):
                self.receipt('[]\n', **changed)

    def test_ambiguous_or_invalid_receipts_fail(self):
        for text in ('[]\n[]\n', '{}\n', '[1]\n', 'no json\n'):
            with self.subTest(text=text), self.assertRaises(ValueError):
                self.receipt(text)

    def test_shell_env_preserves_unusual_paths_without_injection(self):
        value = "/path/with space/'quote;$(exit 3)"
        words = shlex.split(subject.shell_env({'JR_ENGINE_HOME': value, 'CARGO_BUILD_JOBS': '4'}))
        self.assertEqual(words, ['env', 'CARGO_BUILD_JOBS=4', 'JR_ENGINE_HOME=' + value])

    def test_preparation_matches_managed_cargo_home_and_jobs(self):
        args = SimpleNamespace(guest_engine=Path('/guest/engine'),
                               guest_toolchain=Path('/guest/toolchain'),
                               cargo_home=Path('/caller/.cargo'))
        values = subject.preparation_environment(args, {'home': Path('/guest/private')})
        self.assertEqual(values['CARGO_HOME'], Path('/caller/.cargo'))
        self.assertEqual(values['CARGO_BUILD_JOBS'], '4')
        self.assertEqual(values['JR_ENGINE_HOME'], Path('/guest/private'))
        self.assertEqual({k for k in values if k.startswith('CARGO_')},
                         {'CARGO_HOME', 'CARGO_BUILD_JOBS'})

    def test_namespace_probe_is_valid_read_only_python_for_quoted_path(self):
        with tempfile.TemporaryDirectory() as directory:
            command = shlex.split(subject.namespace_probe(Path(directory) / "no worker's home"))
            self.assertEqual(command[:2], ['python3', '-c'])
            self.assertIn("marker=b'JR_ENGINE_HOME='", command[2])
            result = subprocess.run(command, capture_output=True, text=True, timeout=5)
            self.assertEqual((result.returncode, result.stdout.strip()), (0, '[]'), result.stderr)

    def test_summary_requires_complete_accepted_samples_per_route(self):
        rows = [{'stack': stack, 'phase': 'edit', 'wall_seconds': value, 'ok': True}
                for stack in ('shipping', 'hybrid-cold', 'hybrid-warm') for value in (1, 2, 3)]
        self.assertTrue(all(x['accepted'] for x in subject.summarize(rows, 3).values()))
        self.assertEqual(subject.summarize(rows, 3)['hybrid-cold']['median_seconds'], 2)
        rows[-1]['ok'] = False
        value = subject.summarize(rows, 3)['hybrid-warm']
        self.assertFalse(value['accepted'])
        self.assertIsNone(value['median_seconds'])

    def test_no_summary_for_incomplete_route(self):
        values = subject.summarize([], 3)
        self.assertTrue(all(not x['accepted'] and x['median_seconds'] is None for x in values.values()))


if __name__ == '__main__':
    unittest.main()
