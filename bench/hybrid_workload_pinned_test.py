#!/usr/bin/env python3
"""Synthetic pinned-runner validation, not compiler correctness evidence."""
import argparse
import io
import json
from pathlib import Path
import subprocess
import tarfile
import tempfile
import unittest
from unittest.mock import patch

import hybrid_workload_pinned as pinned


NAMES = pinned.resident.NAMES


def transcript(nonce=1, failed=False):
    return '\n'.join('test ' + name + ' ... ' + ('FAILED' if failed and name == pinned.resident.EDIT_TEST else 'ok') for name in NAMES) + '\n' + f'JR_RUNTIME_EXECUTED_{nonce}\n' + ('JR_RUNTIME_FAILURE\n' if failed else '') + f'test result: {"FAILED" if failed else "ok"}. {3 if failed else 4} passed; {1 if failed else 0} failed; 0 ignored; 0 measured; 100 filtered out\n'


FAKE = '''#!/usr/bin/env python3
import os, pathlib, re, sys
if 'stop' in sys.argv: sys.exit(0)
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
    def budget_case(self, fresh=True, live=False, restart=False):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            source = root / 'source'
            (source / 'src').mkdir(parents=True)
            (source / 'src/summary.rs').write_text('fn shares_overlapping_time() {}')
            project = root / 'project'
            project.mkdir()
            home = root / 'home'
            log = home / 'resident-v1/runtime/daemon.log'
            log.parent.mkdir(parents=True)
            event = f'worker x of {project} stopped: BLAZE_MEMORY_MAX asked longest ago\n'
            log.write_text('old\n' + (event if not fresh else ''))
            args = argparse.Namespace(justrust=Path('/fake'), output=root, source=source, timeout=.01)
            local = {'JR_ENGINE_HOME': str(home), 'HYBRID_SIDE_EFFECT': str(root / 'side')}
            prepared = False
            def invoke(command, cwd, env, path, timeout):
                nonlocal prepared
                if 'prewarm' in command:
                    prepared = True
                    if fresh:
                        with log.open('a') as handle:
                            handle.write(event)
                    text = ''
                else:
                    text = 'daemon: pid 10, active\n'
                    if not prepared or live:
                        text += f'  worker: pid 20, {project}, log /fake\n'
                path.write_text(text)
                return {'returncode': 0, 'timeout': False, 'error': None, 'log': str(path)}
            def identity(pid):
                if pid == 20 and prepared and not live:
                    return None
                return {'pid': pid, 'start_ticks': 99 if restart and prepared and pid == 10 else pid, 'state': 'S'}
            with patch.object(pinned.core, 'run', side_effect=invoke), patch.object(pinned, 'worker_identity', side_effect=identity), patch.object(pinned.core, 'namespace_pss', return_value={}):
                return pinned.await_budget_eviction(args, project, local)

    def test_budget_requires_fresh_event_and_dead_worker_same_daemon(self):
        receipt = self.budget_case()
        self.assertTrue(receipt['ok'])
        self.assertTrue(receipt['old_workers_exited'])
        for options in ({'fresh': False}, {'live': True}, {'restart': True}):
            with self.subTest(options=options), self.assertRaisesRegex(ValueError, 'not observed'):
                self.budget_case(**options)

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

    def test_portable_tree_verifies_real_pin_without_checkout_metadata(self):
        repo = Path(__file__).resolve().parent.parent
        archive = subprocess.check_output(['git', 'archive', pinned.resident.PIN], cwd=repo)
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            with tarfile.open(fileobj=io.BytesIO(archive)) as bundle:
                bundle.extractall(root, filter='data')
            self.assertEqual(pinned.source_tree(root), pinned.PIN_TREE)
            pinned.pinned_source(root)
            (root / 'src/summary.rs').write_text('changed')
            with self.assertRaises(ValueError):
                pinned.pinned_source(root)

    def test_portable_tree_rejects_symlinks(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / 'link').symlink_to('/etc/passwd')
            with self.assertRaises(ValueError):
                pinned.source_tree(root)

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
            with patch.object(pinned, 'pinned_source'):
                self.assertEqual(pinned.execute(args), 0)
            report = json.loads((args.output / 'report.json').read_text())
            self.assertEqual(report['status'], 'passed')
            self.assertTrue(report['cleanup_passed'], report['cleanup'])
            self.assertEqual(len(report['rows']), 18 if automatic else 17)
            self.assertEqual((source / 'src/summary.rs').read_bytes(), original)
            self.assertFalse(report['routing_evidence_complete'])
            self.assertTrue(all(Path(row['raw_prefix'] + '.stdout').is_file() if 'raw_prefix' in row else Path(row['log']).is_file() for row in report['rows']))
            if automatic:
                self.assertTrue(report['automatic_interference']['paired_foregrounds_validated'])
                self.assertFalse(report['automatic_interference']['actual_loading_overlap_observed'])

    def test_synthetic_end_to_end(self):
        self.run_matrix()

    def test_synthetic_automatic_prewarm(self):
        self.run_matrix(automatic=True)


if __name__ == '__main__':
    unittest.main()
